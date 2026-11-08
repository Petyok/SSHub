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
    for x in &d {
        crate::sync::ceremony::test_force(&x.store, false);
    }
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
    let reg = propose(&d, R, factor_register_paper(&phrase).unwrap());
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
    assert!(err.contains("big change"), "{err}");
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
    let reg = propose(&d, R, factor_register_paper(&phrase).unwrap());
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
    let reg = propose(&d, R, factor_register_paper(&phrase).unwrap());
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

// app round 2, F5: a dead email factor says why.
#[test]
fn the_ceremony_says_when_the_mailbox_sender_made_the_emailed_code_unusable() {
    let d = world(&[R, A, B]);
    let phrase = bip39_encode(&[0x34; 32]);
    let reg = propose(&d, R, factor_register_paper(&phrase).unwrap());
    append(&d[P], red_phone_sign_body(&reg));
    append(&d[A], red_approve_body(&reg));
    sync(&d);
    for x in &d {
        settle_red_ops(&x.store, reg.timestamp + 5).unwrap();
    }
    let recover = paper_derive(&phrase, PaperDomain::Recover).unwrap();
    let (body, _code) = factor_register_email(&d[B].did).unwrap();
    let ereg = propose(&d, R, body);
    append(&d[P], red_phone_sign_body(&ereg));
    append(&d[A], red_approve_body(&ereg));
    append(&d[R], red_factor_paper(&recover, &ereg));
    sync(&d);
    for x in &d {
        settle_red_ops(&x.store, ereg.timestamp + 5).unwrap();
    }
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
    // The sender is the one who gave the second-device vote: its own code cannot be a separate factor.
    append(&d[B], red_approve_body(&op));
    sync(&d);
    let st = status(&d, &op, op.timestamp + 30);
    assert!(
        st.note.as_deref().is_some_and(|n| n.contains("sender")),
        "{st:?}"
    );
    assert!(!st.complete);
}

#[test]
fn the_emailed_code_is_a_factor_and_its_sender_mails_only_when_the_other_proofs_are_in() {
    let d = world(&[R, A, B]);
    let phrase = bip39_encode(&[0x34; 32]);
    let recover = paper_derive(&phrase, PaperDomain::Recover).unwrap();
    let reg = propose(&d, R, factor_register_paper(&phrase).unwrap());
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
    remember_email_code(&d[B].store, &d[B].id, &code).unwrap();
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
        owed_code_mails(&d[B].store, &d[B].id, now)
            .unwrap()
            .is_empty(),
        "the phone and the second device have not voted yet"
    );
    append(&d[P], red_phone_sign_body(&op));
    append(&d[A], red_approve_body(&op));
    sync(&d);
    assert!(
        owed_code_mails(&d[A].store, &d[A].id, now)
            .unwrap()
            .is_empty(),
        "only the sender mails"
    );
    let mails = owed_code_mails(&d[B].store, &d[B].id, now).unwrap();
    assert_eq!(mails.len(), 1);
    assert!(mails[0].2.contains(&code) && mails[0].2.contains("opens nothing"));
    // A failing mail server marks nothing; a working one marks it, and then nothing is owed.
    let mut failing = |_: &str, _: &str| -> Result<()> { anyhow::bail!("no mta") };
    assert_eq!(
        send_owed_codes(&d[B].store, &d[B].id, now, &mut failing).unwrap(),
        0
    );
    assert_eq!(
        owed_code_mails(&d[B].store, &d[B].id, now).unwrap().len(),
        1
    );
    let got = std::cell::RefCell::new(Vec::new());
    let mut ok = |s: &str, b: &str| -> Result<()> {
        got.borrow_mut().push((s.to_string(), b.to_string()));
        Ok(())
    };
    assert_eq!(
        send_owed_codes(&d[B].store, &d[B].id, now, &mut ok).unwrap(),
        1
    );
    assert!(got.borrow()[0].1.contains(&code));
    assert_eq!(
        send_owed_codes(&d[B].store, &d[B].id, now, &mut ok).unwrap(),
        0
    );
    assert!(
        owed_code_mails(&d[B].store, &d[B].id, now)
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

// ---------------------------------------------------------------- the ceremony-complete rule (contract 4g)

mod in_force {
    use super::*;
    use crate::sync::crypto::{bip39_encode, Argon2Profile};
    use crate::sync::quorum::tests::{append, complete, ring_phone};
    use crate::sync::quorum::{live_holder_count, paper_pub, paper_registration, share_recipients};

    const TEST: Argon2Profile = Argon2Profile {
        mem_kib: 64,
        iters: 1,
    };

    /// Three terminals (0, 1, 2) and the phone (3), the paper registered through a completed ceremony.
    fn ring_with_paper() -> (Vec<Dev>, String) {
        let d = ring_phone(3);
        let phrase = bip39_encode(&[0x77; 32]);
        let reg = append(&d[0], paper_registration(&phrase).unwrap());
        complete(&d, &reg, 1, None);
        (d, phrase)
    }

    #[test]
    fn a_paper_registration_changes_who_is_wrapped_to_only_when_complete() {
        let d = ring_phone(3);
        let phrase = bip39_encode(&[0x77; 32]);
        let reg = append(&d[0], paper_registration(&phrase).unwrap());
        assert!(
            paper_pub(&d[0].store).unwrap().is_none(),
            "the proposer too"
        );
        let n = share_recipients(&d[0].store, &d[0].id.public())
            .unwrap()
            .len();
        complete(&d, &reg, 1, None);
        for x in &d {
            assert!(paper_pub(&x.store).unwrap().is_some());
        }
        assert_eq!(
            share_recipients(&d[0].store, &d[0].id.public())
                .unwrap()
                .len(),
            n + 1
        );
    }

    #[test]
    fn an_exemption_wraps_nothing_until_complete() {
        let (d, phrase) = ring_with_paper();
        let ex = append(
            &d[0],
            crate::sync::exemption::enable_body(&d[0].id, "some words here", TEST),
        );
        all_to_all_pull(&d);
        for x in &d {
            assert!(crate::sync::exemption::active_slots(&x.store)
                .unwrap()
                .is_empty());
        }
        complete(&d, &ex, 1, Some(&phrase));
        for x in &d {
            assert_eq!(
                crate::sync::exemption::active_slots(&x.store)
                    .unwrap()
                    .len(),
                1
            );
        }
    }

    #[test]
    fn a_promotion_gets_no_shares_until_complete() {
        let (d, phrase) = ring_with_paper();
        let who = d[2].did.clone();
        let promo = append(
            &d[0],
            OpBody::DeviceRoleChange {
                device_id: who.clone(),
                to: DeviceRole::Holder,
            },
        );
        let has = |x: &Dev| {
            share_recipients(&x.store, &x.id.public())
                .unwrap()
                .iter()
                .any(|(id, _)| *id == who)
        };
        assert!(!has(&d[0]), "not a recipient on the proposer");
        assert!(crate::sync::reshare::reissue_plan(
            &d[0].store,
            &d[0].id,
            &[d[0].did.clone(), d[1].did.clone(), who.clone()],
            1
        )
        .is_err());
        complete(&d, &promo, 1, Some(&phrase));
        assert!(has(&d[0]) && has(&d[1]));
    }

    #[test]
    fn a_certificate_counts_nowhere_until_complete() {
        use crate::sync::cert::cert_allows;
        use crate::sync::oplog::CertScope;
        let (d, phrase) = ring_with_paper();
        let phone = &d[3];
        let host = [41u8; 16];
        let now = crate::store::now_ts();
        let op = append(
            phone,
            OpBody::CertIssue {
                device_id: d[1].did.clone(),
                scope: CertScope::Hosts(vec![host]),
                expiry: now + 3600,
                max_new_hosts_per_day: 2,
                epoch_lo: 0,
                epoch_hi: 10,
            },
        );
        all_to_all_pull(&d);
        let cert = crate::sync::cert::DelegationCert::from_op(&op).unwrap();
        let pk = phone.id.public().ed25519_pub;
        let at = op.timestamp + 5;
        for x in &d[..3] {
            let err = cert_allows(&cert, &pk, &x.store, &d[1].did, &host, at, at).unwrap_err();
            assert!(err.to_string().contains("not approved yet"), "{err}");
        }
        complete(&d, &op, 2, Some(&phrase));
        for x in &d[..3] {
            cert_allows(&cert, &pk, &x.store, &d[1].did, &host, at, at).unwrap();
        }
    }

    #[test]
    fn a_revocation_reshares_nothing_until_complete() {
        use crate::sync::revocation::next_actions;
        let (d, phrase) = ring_with_paper();
        let target = d[2].did.clone();
        let holders = live_holder_count(&d[0].store).unwrap();
        let rev = append(&d[0], OpBody::DeviceRevoke { device_id: target });
        let now = crate::store::now_ts();
        assert_eq!(
            live_holder_count(&d[0].store).unwrap(),
            holders,
            "still a holder"
        );
        assert!(next_actions(&d[0].store, &d[0].id, now).unwrap().is_empty());
        complete(&d, &rev, 1, Some(&phrase));
        assert_eq!(live_holder_count(&d[0].store).unwrap(), holders - 1);
        assert!(!next_actions(&d[0].store, &d[0].id, now).unwrap().is_empty());
    }

    #[test]
    fn an_export_window_opens_only_when_complete() {
        let (d, phrase) = ring_with_paper();
        let op = append(
            &d[1],
            OpBody::BulkExport {
                requester: d[1].did.clone(),
                window_secs: 600,
            },
        );
        let at = op.timestamp + 1;
        assert!(
            crate::sync::export::active_window(&d[1].store, &d[1].did, at)
                .unwrap()
                .is_none()
        );
        complete(&d, &op, 0, Some(&phrase));
        assert!(
            crate::sync::export::active_window(&d[1].store, &d[1].did, at)
                .unwrap()
                .is_some()
        );
    }

    /// Ingest everything everywhere without clicking approve (the ceremony decides, not the fixture).
    fn all_to_all_pull(d: &[Dev]) {
        for _ in 0..2 {
            for a in d {
                for b in d {
                    if a.did != b.did {
                        pull(a, b);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- review round 1 regressions

mod review {
    use super::*;
    use crate::sync::cert::{cert_allows, cert_opened_body, DelegationCert};
    use crate::sync::cred_ops::cred_put;
    use crate::sync::crypto::{Argon2Profile, DeviceIdentity};
    use crate::sync::oplog::{CertScope, DeviceKind, HostUid, VerifiedBy};
    use crate::sync::protocol::tests::ring;
    use crate::sync::quorum::tests::{
        all_to_all, complete, deliver, payload, quorum_mode, ring_phone,
    };
    use crate::sync::quorum::{assemble_dek, own_shares, phone_device};
    use crate::sync::reshare::reissue_receive;
    use crate::sync::revocation::{next_actions, perform, RevocationAction};
    use std::sync::Arc;

    const TEST: Argon2Profile = Argon2Profile {
        mem_kib: 64,
        iters: 1,
    };

    fn sync_pull(d: &[Dev]) {
        for _ in 0..2 {
            for a in d {
                for b in d {
                    if a.did != b.did {
                        pull(a, b);
                    }
                }
            }
        }
    }

    fn mirror_in(d: &[Dev]) -> Dev {
        let m = Dev::new();
        for x in d.iter().chain(std::iter::once(&m)) {
            x.store
                .sync_register_device(
                    &m.id.public(),
                    "mirror",
                    DeviceRole::Mirror,
                    Attendance::Attended,
                    DeviceKind::Terminal,
                    VerifiedBy::Sas,
                )
                .unwrap();
        }
        for x in d {
            m.trust(x);
        }
        m
    }

    // crypto F1: the delegate cannot waive the daily budget by recording hosts as opened itself.
    #[test]
    fn the_delegate_recording_opened_hosts_does_not_waive_the_new_host_budget() {
        let d = ring(4); // 0 phone, 1 delegate, 2 and 3 enforcing holders
        let hosts: Vec<HostUid> = (0..5u8).map(|n| [200 + n; 16]).collect();
        let now = crate::store::now_ts();
        let op = append(
            &d[0],
            OpBody::CertIssue {
                device_id: d[1].did.clone(),
                scope: CertScope::Hosts(hosts.clone()),
                expiry: now + 7 * 86_400,
                max_new_hosts_per_day: 1,
                epoch_lo: 0,
                epoch_hi: 10,
            },
        );
        all_to_all(&d);
        let cert = DelegationCert::from_op(&op).unwrap();
        let pk = d[0].id.public().ed25519_pub;
        let at = op.timestamp + 10;
        for h in &hosts {
            append(&d[1], cert_opened_body(&cert, h));
        }
        all_to_all(&d);
        // The delegate's own records buy nothing: holders release host by host, each release recorded by
        // the holder that made it, and the budget of one new host a day is spent by the first.
        let mut allowed = 0;
        for h in &hosts {
            if cert_allows(&cert, &pk, &d[2].store, &d[1].did, h, at, at).is_ok() {
                allowed += 1;
                append(&d[2], cert_opened_body(&cert, h));
                all_to_all(&d);
            }
        }
        assert_eq!(allowed, 1, "{allowed} hosts got through a budget of 1");
        // A holder's record does count: the host it opened is free afterwards, on any holder.
        assert!(cert_allows(&cert, &pk, &d[3].store, &d[1].did, &hosts[0], at, at).is_ok());
        assert!(cert_allows(&cert, &pk, &d[3].store, &d[1].did, &hosts[1], at, at).is_err());
    }

    fn drive_reshare(d: &mut [Dev], now: i64, perform_receive: bool) {
        for _ in 0..6 {
            for dev in d.iter_mut().take(3) {
                for a in next_actions(&dev.store, &dev.id, now).unwrap() {
                    if matches!(a, RevocationAction::Retire)
                        || (!perform_receive && matches!(a, RevocationAction::Receive { .. }))
                    {
                        continue;
                    }
                    let store = dev.store.clone();
                    let bodies = perform(&store, Arc::get_mut(&mut dev.id).unwrap(), &a).unwrap();
                    for b in bodies {
                        append(dev, b);
                    }
                }
            }
            all_to_all(&d[..3]);
        }
    }

    // crypto F2: a mirror cannot close an open revocation with an empty reshare batch.
    #[test]
    fn a_non_holder_cannot_close_a_revocation_with_a_reshare_batch() {
        let d = ring(4);
        quorum_mode(&d[0]);
        all_to_all(&d);
        let m = mirror_in(&d);
        let now = crate::store::now_ts();
        append(
            &d[0],
            OpBody::DeviceRevoke {
                device_id: d[3].did.clone(),
            },
        );
        all_to_all(&d[..3]);
        for x in &d[..3] {
            deliver(x, &m);
        }
        let mut holders: Vec<_> = d[..3].iter().map(|x| x.did.clone()).collect();
        holders.sort();
        append(
            &m,
            OpBody::ShareReshareBatch {
                epoch: 1,
                new_holders: holders,
                items: vec![],
            },
        );
        for x in &d[..3] {
            deliver(&m, x);
        }
        let actions = next_actions(&d[0].store, &d[0].id, now).unwrap();
        assert!(actions.contains(&RevocationAction::Rotate), "{actions:?}");
    }

    // crypto F3: a mirror's batch cannot make holders disagree, and the previous wrap keys are never
    // retired on a timer while credentials are still unadopted.
    #[test]
    fn a_non_holder_batch_cannot_poison_a_reshare_and_wrap_keys_are_retired_only_after_adoption() {
        let mut d = ring(4);
        quorum_mode(&d[0]);
        all_to_all(&d);
        for n in 1..=2u8 {
            cred_put(&d[0].store, &d[0].id, &[100 + n; 16], &payload("x")).unwrap();
        }
        all_to_all(&d);
        let m = mirror_in(&d);
        let now = crate::store::now_ts();
        append(
            &d[0],
            OpBody::DeviceRevoke {
                device_id: d[3].did.clone(),
            },
        );
        all_to_all(&d[..3]);
        drive_reshare(&mut d, now, false);
        assert!(
            reissue_receive(&d[0].store, &d[0].id, 1).is_ok(),
            "honest baseline"
        );
        for x in &d[..3] {
            deliver(x, &m);
        }
        append(
            &m,
            OpBody::ShareReshareBatch {
                epoch: 1,
                new_holders: vec![d[0].did.clone()],
                items: vec![],
            },
        );
        for x in &d[..3] {
            deliver(&m, x);
        }
        for x in &d[..3] {
            assert!(reissue_receive(&x.store, &x.id, 1).is_ok());
        }
        // Everyone received; nobody has adopted by a real unlock yet: thirty days later nothing is retired.
        drive_reshare(&mut d, now, true);
        let later = now + 30 * 86_400;
        for x in &d[..3] {
            assert!(
                !next_actions(&x.store, &x.id, later)
                    .unwrap()
                    .contains(&RevocationAction::Retire),
                "wrap keys retired on a timer"
            );
        }
    }

    // crypto F4a: a device of kind Phone added as a mirror is a Red op and never the ring's phone.
    #[test]
    fn a_phone_kind_device_added_as_a_mirror_is_red_and_never_supplies_the_phone_proof() {
        let d = ring(3);
        for x in &d {
            crate::sync::ceremony::test_force(&x.store, false);
        }
        let m = Dev::new();
        crate::sync::ceremony::test_force(&m.store, false);
        let add = append(
            &d[0],
            OpBody::DeviceAdd {
                device: m.id.public(),
                name: "phone".into(),
                role: DeviceRole::Mirror,
                attendance: Attendance::Attended,
                kind: DeviceKind::Phone,
                verified_by: VerifiedBy::Sas,
            },
        );
        assert!(is_gated(&d[0].store, &add).unwrap());
        sync_pull(&d);
        assert!(resolve(&d[1].store, &add.hash(), true, add.timestamp + 5).is_err());
        assert!(phone_device(&d[1].store).unwrap().is_none());
        assert!(phone_device(&d[0].store).unwrap().is_none());
    }

    // crypto F4b: a second phone is a verdict on its own DeviceAdd, not an error in every evaluation.
    #[test]
    fn a_second_phone_is_blocked_and_does_not_break_other_ceremonies() {
        let d = ring_phone(3);
        let m = Dev::new();
        let add = append(
            &d[0],
            OpBody::DeviceAdd {
                device: m.id.public(),
                name: "phone two".into(),
                role: DeviceRole::Holder,
                attendance: Attendance::Attended,
                kind: DeviceKind::Phone,
                verified_by: VerifiedBy::Sas,
            },
        );
        sync_pull(&d);
        let st = red_status(&d[2].store, &add, add.timestamp + 1).unwrap();
        assert!(
            st.blocked.is_some() && st.rule_violation.is_some(),
            "{st:?}"
        );
        let probe = append(
            &d[1],
            OpBody::BulkExport {
                requester: d[1].did.clone(),
                window_secs: 60,
            },
        );
        sync_pull(&d);
        red_status(&d[2].store, &probe, probe.timestamp + 1).unwrap();
        assert_eq!(phone_device(&d[2].store).unwrap(), Some(d[3].did.clone()));
    }

    fn quorum_ring_with_paper() -> (Vec<Dev>, String) {
        let d = ring_phone(3);
        let phrase = bip39_encode(&[0x77; 32]);
        let reg = append(
            &d[0],
            crate::sync::quorum::paper_registration(&phrase).unwrap(),
        );
        complete(&d, &reg, 1, None);
        let mc = append(
            &d[0],
            OpBody::ModeChange {
                to: CredMode::Quorum,
                migrated_on: d[0].did.clone(),
                cosigner: None,
            },
        );
        complete(&d, &mc, 1, Some(&phrase));
        (d, phrase)
    }

    // crypto F5: only the exempt device makes its own slot; a slot chosen by another proposer, or one that
    // equals a known key, never becomes active even with every proof present.
    #[test]
    fn an_exemption_slot_is_active_only_when_the_exempt_device_made_it() {
        let (d, phrase) = quorum_ring_with_paper();
        let made_up = DeviceIdentity::generate();
        let bad = append(
            &d[0],
            OpBody::ExemptionSet {
                device_id: d[2].did.clone(),
                slot: Some(made_up.public()),
            },
        );
        complete(&d, &bad, 1, Some(&phrase));
        for x in &d {
            assert!(crate::sync::exemption::active_slots(&x.store)
                .unwrap()
                .is_empty());
        }
        let host = [9u8; 16];
        cred_put(&d[0].store, &d[0].id, &host, &payload("secret")).unwrap();
        all_to_all(&d);
        assert!(own_shares(&d[0].store, &made_up, &host).is_err());
        let mine = own_shares(&d[0].store, &d[0].id, &host)
            .unwrap()
            .remove(0)
            .1;
        assert!(assemble_dek(&d[0].store, &host, mine.clone(), mine).is_err());

        // A slot equal to another device's key is refused too.
        let clash = append(
            &d[2],
            OpBody::ExemptionSet {
                device_id: d[2].did.clone(),
                slot: Some(d[1].id.public()),
            },
        );
        complete(&d, &clash, 0, Some(&phrase));
        assert!(crate::sync::exemption::active_slots(&d[0].store)
            .unwrap()
            .is_empty());

        // The honest path: the exempt device derives and proposes its own slot.
        let ok = append(
            &d[2],
            crate::sync::exemption::enable_body(
                &d[2].id,
                "alpha bravo charlie delta echo foxtrot",
                TEST,
            ),
        );
        complete(&d, &ok, 0, Some(&phrase));
        for x in &d {
            assert_eq!(
                crate::sync::exemption::active_slots(&x.store)
                    .unwrap()
                    .len(),
                1
            );
        }
        let _ = ok;
    }

    // crypto F6: a Red op stamped in the future expires like any other.
    #[test]
    fn a_future_dated_red_op_expires_like_any_other() {
        let d = ring_phone(3);
        let phrase = bip39_encode(&[0x66; 32]);
        let now = crate::store::now_ts();
        let reg = d[0]
            .store
            .sync_append_local_at(
                &d[0].id,
                crate::sync::quorum::paper_registration(&phrase).unwrap(),
                now + 5 * 365 * 86_400,
            )
            .unwrap();
        sync_pull(&d);
        append(&d[3], red_phone_sign_body(&reg));
        append(&d[1], red_approve_body(&reg));
        sync_pull(&d);
        let st = red_status(&d[2].store, &reg, now + 400 * 86_400).unwrap();
        assert!(st.expired && !st.complete, "{st:?}");
    }

    // crypto F7a: the emailed code is never in clear in sync_meta, and only this device's key opens it.
    #[test]
    fn the_email_code_is_sealed_under_the_device_key() {
        let id = DeviceIdentity::generate();
        let other = DeviceIdentity::generate();
        let (_, code) = factor_register_email(&id.public().device_id()).unwrap();
        let store = LauncherStore::open_in_memory().unwrap();
        remember_email_code(&store, &id, &code).unwrap();
        assert!(meta_get(&store, "email_factor_code").unwrap().is_none());
        let raw = meta_get(&store, CODE_KEY).unwrap().unwrap();
        assert!(!raw.windows(code.len()).any(|w| w == code.as_bytes()));
        assert_eq!(
            stored_code(&store, &id).unwrap().as_deref(),
            Some(code.as_str())
        );
        assert!(stored_code(&store, &other).unwrap().is_none());
    }

    // crypto F7b and app F3: the registered sender neither mails for an op it approved nor finishes it
    // alone with the code it holds.
    #[test]
    fn the_code_sender_that_is_the_second_approver_mails_nothing_and_cannot_finish_alone() {
        let d = ring_phone(3);
        let phrase = bip39_encode(&[0x34; 32]);
        let reg = append(
            &d[0],
            crate::sync::quorum::paper_registration(&phrase).unwrap(),
        );
        complete(&d, &reg, 2, None);
        let (body, code) = factor_register_email(&d[1].did).unwrap();
        let ereg = append(&d[0], body);
        complete(&d, &ereg, 2, Some(&phrase));
        remember_email_code(&d[1].store, &d[1].id, &code).unwrap();
        let op = append(
            &d[0],
            OpBody::BulkExport {
                requester: d[0].did.clone(),
                window_secs: 60,
            },
        );
        sync_pull(&d);
        append(&d[3], red_phone_sign_body(&op));
        append(&d[1], red_approve_body(&op)); // the sender gives the second vote
        sync_pull(&d);
        let now = op.timestamp + 30;
        assert!(owed_code_mails(&d[1].store, &d[1].id, now)
            .unwrap()
            .is_empty());
        append(&d[1], red_factor_email(&d[1].store, &code, &op).unwrap());
        sync_pull(&d);
        assert!(!red_status(&d[2].store, &op, now).unwrap().complete);
        // A different second device and the sender mailing: the ordinary path still works and the mail
        // says what the change is.
        let op2 = append(
            &d[0],
            OpBody::BulkExport {
                requester: d[0].did.clone(),
                window_secs: 90,
            },
        );
        sync_pull(&d);
        append(&d[3], red_phone_sign_body(&op2));
        append(&d[2], red_approve_body(&op2));
        sync_pull(&d);
        let mails = owed_code_mails(&d[1].store, &d[1].id, op2.timestamp + 30).unwrap();
        assert_eq!(mails.len(), 1);
        assert!(
            mails[0].2.contains("export every credential for 90 s"),
            "{}",
            mails[0].2
        );
        append(&d[2], red_factor_email(&d[1].store, &code, &op2).unwrap());
        sync_pull(&d);
        assert!(
            red_status(&d[2].store, &op2, op2.timestamp + 40)
                .unwrap()
                .complete
        );
    }

    // The note red_status sets reaches the review's view (TUI and sync-review read RedView).
    #[test]
    fn the_note_of_red_status_travels_to_the_review_view() {
        let d = ring_phone(3);
        let phrase = bip39_encode(&[0x35; 32]);
        let reg = append(
            &d[0],
            crate::sync::quorum::paper_registration(&phrase).unwrap(),
        );
        complete(&d, &reg, 2, None);
        let (body, code) = factor_register_email(&d[1].did).unwrap();
        let ereg = append(&d[0], body);
        complete(&d, &ereg, 2, Some(&phrase));
        remember_email_code(&d[1].store, &d[1].id, &code).unwrap();
        let op = append(
            &d[0],
            OpBody::BulkExport {
                requester: d[0].did.clone(),
                window_secs: 60,
            },
        );
        sync_pull(&d);
        append(&d[3], red_phone_sign_body(&op));
        append(&d[1], red_approve_body(&op)); // the sender gives the second vote
        sync_pull(&d);
        let st = red_status(&d[2].store, &op, op.timestamp + 30).unwrap();
        let note = st
            .note
            .clone()
            .expect("red_status explains the dead email factor");
        let rows = d[2].store.sync_devices().unwrap();
        let view = crate::app::sync_view::pending_view(&d[2].store, &op, &rows, None);
        assert_eq!(view.red.expect("a Red view").note, Some(note));
    }

    // crypto F8: the phone looks as fresh as what it signed, not as the day a courier delivered it.
    #[test]
    fn phone_freshness_follows_what_the_phone_signed() {
        let d = ring(3);
        let now = crate::store::now_ts();
        let old = now - 30 * 86_400;
        let op = d[0]
            .store
            .sync_append_local_at(
                &d[0].id,
                OpBody::HeadAttest {
                    device_id: d[0].did.clone(),
                    seq: 0,
                    chain_hash: [0; 32],
                },
                old,
            )
            .unwrap();
        pull(&d[0], &d[2]);
        let seen = crate::sync::quorum::phone_signed_at(&d[2].store, &d[0].did, now).unwrap();
        assert!(seen <= op.timestamp + 5, "{seen} vs {}", op.timestamp);
    }

    // crypto round 2, F1: a proof is as old as its receipt, whatever stamp its author chose.
    #[test]
    fn a_backdated_late_factor_does_not_complete_a_stale_ceremony() {
        let d = ring_phone(3);
        let phrase = bip39_encode(&[0x31; 32]);
        let reg = append(
            &d[0],
            crate::sync::quorum::paper_registration(&phrase).unwrap(),
        );
        complete(&d, &reg, 1, None);
        let now = crate::store::now_ts();
        let t0 = now - 10 * 86_400;
        let op = d[0]
            .store
            .sync_append_local_at(
                &d[0].id,
                OpBody::BulkExport {
                    requester: d[0].did.clone(),
                    window_secs: 60,
                },
                t0,
            )
            .unwrap();
        sync_pull(&d);
        d[3].store
            .sync_append_local_at(&d[3].id, red_phone_sign_body(&op), t0 + 3_600)
            .unwrap();
        d[1].store
            .sync_append_local_at(&d[1].id, red_approve_body(&op), t0 + 3_600)
            .unwrap();
        sync_pull(&d);
        assert!(!in_force(&d[0].store, &op).unwrap(), "no factor yet");
        let recover = paper_derive(&phrase, PaperDomain::Recover).unwrap();
        d[2].store
            .sync_append_local_at(&d[2].id, red_factor_paper(&recover, &op), now)
            .unwrap();
        sync_pull(&d);
        assert!(
            !in_force(&d[0].store, &op).unwrap(),
            "control: a late proof is refused"
        );
        d[1].store
            .sync_append_local_at(&d[1].id, red_factor_paper(&recover, &op), t0 + 7_200)
            .unwrap();
        sync_pull(&d);
        assert!(
            !in_force(&d[0].store, &op).unwrap(),
            "a factor proof received ten days late completed the ceremony because its stamp is backdated"
        );
    }

    // crypto round 2, F3: only a holder's grant answers a share request.
    #[test]
    fn a_grant_from_a_non_holder_does_not_pre_empt_the_real_one() {
        let d = ring(3);
        quorum_mode(&d[0]);
        all_to_all(&d);
        let host = [77u8; 16];
        cred_put(&d[0].store, &d[0].id, &host, &payload("p")).unwrap();
        all_to_all(&d);
        let m = mirror_in(&d);
        for x in &d {
            deliver(&m, x);
            deliver(x, &m);
        }
        let req = crate::sync::quorum::peer_request(&d[0].store, &d[0].id, &host).unwrap();
        let OpBody::ShareRelease { eph_x25519_pub, .. } = &req.op.body else {
            unreachable!()
        };
        deliver(&d[0], &m);
        append(
            &m,
            OpBody::ShareGrant {
                host_uid: host,
                request_id: req.request_id,
                requester: d[0].did.clone(),
                epoch: 0,
                sealed: crate::sync::share_release::seal_share_response(
                    eph_x25519_pub,
                    &crate::sync::crypto::Share {
                        index: 2,
                        bytes: vec![1; 32],
                    },
                ),
            },
        );
        deliver(&m, &d[0]);
        assert!(
            crate::sync::quorum::peer_poll(&d[0].store, &req, 0)
                .unwrap()
                .is_none(),
            "a grant written by a mirror was handed to the unlocker as the holder's share"
        );
    }

    // protocol S5/app F11: foreign names are plain text in mails and screens.
    #[test]
    fn names_in_mails_carry_no_control_characters() {
        assert_eq!(plain_text("evil\nBcc: x\u{1b}[2K\r."), "evilBcc: x[2K.");
    }
}

// ------------------------------------------------ phone admission through a CLI peer

/// A ring of two terminals (proposer `r`, second holder `a`) and a phone that only the proposer
/// (and the phone itself) knows: the state right after `sync-phone-enroll` on the proposer.
fn phone_admission_world() -> (Dev, Dev, Dev, Op) {
    let (r, a, ph) = (Dev::new(), Dev::new(), Dev::new());
    for x in [&r, &a, &ph] {
        crate::sync::ceremony::test_force(&x.store, false);
    }
    for x in [&r, &a] {
        for y in [&r, &a] {
            x.store
                .sync_register_device(
                    &y.id.public(),
                    "terminal",
                    DeviceRole::Holder,
                    Attendance::Attended,
                    DeviceKind::Terminal,
                    VerifiedBy::Sas,
                )
                .unwrap();
        }
    }
    // What `admit_phone` does on the proposer: register the phone locally, propose its admission.
    r.store
        .sync_register_device(
            &ph.id.public(),
            "phone",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Phone,
            VerifiedBy::Sas,
        )
        .unwrap();
    ph.store
        .sync_register_device(
            &r.id.public(),
            "terminal",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    let add = append(
        &r,
        OpBody::DeviceAdd {
            device: ph.id.public(),
            name: "phone".into(),
            role: DeviceRole::Holder,
            attendance: Attendance::Attended,
            kind: DeviceKind::Phone,
            verified_by: VerifiedBy::Sas,
        },
    );
    (r, a, ph, add)
}

#[test]
fn a_phone_is_admitted_on_a_peer_that_never_heard_of_it() {
    let (r, a, ph, add) = phone_admission_world();
    let known = |x: &Dev| {
        x.store
            .sync_devices()
            .unwrap()
            .iter()
            .any(|d| d.id == ph.did)
    };
    assert!(!known(&a), "the second holder starts without the phone");
    // The phone sees the notice and signs for its own admission; the proposer carries both.
    pull(&r, &ph);
    append(&ph, red_phone_sign_body(&add));
    pull(&ph, &r);
    pull(&r, &a);
    pull(&a, &r);
    // The second holder holds the DeviceAdd (pending) and the phone's proof (an unknown author).
    assert!(a
        .store
        .sync_pending()
        .unwrap()
        .iter()
        .any(|o| o.hash() == add.hash()));
    let proof_row_unknown = a
        .store
        .sync_pending_with_reasons()
        .unwrap()
        .iter()
        .any(|(o, why)| o.device_id == ph.did && why == "unknown-author");
    assert!(
        proof_row_unknown,
        "the phone's proof waits as an unknown author"
    );
    // The ceremony counts the phone's proof for its own admission, from the key in the DeviceAdd.
    append(&a, red_approve_body(&add));
    let now = add.timestamp + 60;
    let st = red_status(&a.store, &add, now).unwrap();
    assert!(st.have.contains(&RedProof::Phone), "{st:?}");
    assert!(st.have.contains(&RedProof::SecondDevice), "{st:?}");
    assert!(st.complete, "{st:?}");
    assert_eq!(
        settle_red_ops(&a.store, now).unwrap().applied,
        vec![add.hash()]
    );
    assert!(known(&a), "the phone is a device of the second holder now");
    // Its later ops are known-author everywhere: seq 1 (the proof) is released, seq 2 is plain.
    let later = append(&ph, red_phone_sign_body(&add));
    pull(&ph, &r);
    pull(&r, &a);
    let ops = a.store.sync_chain_tail(&ph.did, 0).unwrap();
    assert!(
        ops.iter().any(|o| o.hash() == later.hash()),
        "the phone's later op is part of its chain at the second holder"
    );
    assert!(a
        .store
        .sync_pending_with_reasons()
        .unwrap()
        .iter()
        .all(|(o, _)| o.device_id != ph.did));
}

#[test]
fn a_proof_for_its_own_admission_is_checked_against_the_key_in_the_device_add() {
    let (r, a, ph, add) = phone_admission_world();
    // A stranger claims the phone's id: its signature is not by the key the DeviceAdd carries.
    let stranger = Dev::new();
    stranger
        .store
        .sync_register_device(
            &r.id.public(),
            "t",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    pull(&r, &stranger);
    let mut forged = append(&stranger, red_phone_sign_body(&add));
    forged.device_id = ph.did.clone();
    forged.seq = 1;
    forged.signature = stranger.id.sign(&forged.canonical_bytes());
    pull(&r, &a);
    a.store.sync_ingest(forged).unwrap();
    append(&a, red_approve_body(&add));
    let st = red_status(&a.store, &add, add.timestamp + 60).unwrap();
    assert!(!st.have.contains(&RedProof::Phone), "{st:?}");
    // And a genuine proof for ANOTHER op does not count for this one.
    let other = append(
        &r,
        OpBody::ModeChange {
            to: CredMode::Quorum,
            migrated_on: r.did.clone(),
            cosigner: None,
        },
    );
    pull(&r, &ph);
    append(&ph, red_phone_sign_body(&other));
    pull(&ph, &r);
    pull(&r, &a);
    let st = red_status(&a.store, &add, add.timestamp + 60).unwrap();
    assert!(!st.have.contains(&RedProof::Phone), "{st:?}");
}

// round 3 (integration): the factor-free first phone is the SETUP of a ring, never a way back in.
#[test]
fn the_factor_free_first_phone_is_ring_setup_only() {
    let (r, a, ph, add) = phone_admission_world();
    pull(&r, &ph);
    append(&ph, red_phone_sign_body(&add));
    pull(&ph, &r);
    pull(&r, &a);
    pull(&a, &r);
    append(&a, red_approve_body(&add));
    let now = add.timestamp + 60;
    let st = red_status(&a.store, &add, now).unwrap();
    assert!(st.complete, "ring setup: no factor, no phone ever: {st:?}");
    // The ring had a phone before (revoked since): a new one needs the third factor like any Red change.
    let old = Dev::new();
    a.store
        .sync_register_device(
            &old.id.public(),
            "old phone",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Phone,
            VerifiedBy::Sas,
        )
        .unwrap();
    a.store
        .with_conn(|c| {
            c.execute(
                "UPDATE sync_devices SET status = 'revoked' WHERE device_id = ?1",
                rusqlite::params![old.did],
            )?;
            Ok(())
        })
        .unwrap();
    let st = red_status(&a.store, &add, now).unwrap();
    assert!(
        st.missing.iter().any(|m| matches!(m, RedProof::Factor(_))),
        "a revoked phone does not reopen the bootstrap: {st:?}"
    );
}

// ----------------------------------------------------------------- ring setup (contract 4h)

fn setup_register(on: &Dev, who: &Dev, role: DeviceRole, kind: DeviceKind) {
    on.store
        .sync_register_device(
            &who.id.public(),
            "peer",
            role,
            Attendance::Attended,
            kind,
            VerifiedBy::Sas,
        )
        .unwrap();
}

/// A ring from a fresh install, step by step as the CLI builds it: one founder holder, joiners that are
/// mirrors, no factor, no credential. The phone and the second terminal holder are each other's missing proof
/// unless the ring is in setup; once the ring has a phone and two terminal holders, setup is over for good.
#[test]
fn a_ring_from_a_fresh_install_reaches_a_phone_and_two_holders_and_setup_then_ends() {
    let (a, b, c, ph) = (Dev::new(), Dev::new(), Dev::new(), Dev::new());
    for x in [&a, &b, &c, &ph] {
        crate::sync::ceremony::test_force(&x.store, false);
    }
    // a is the founder and the only holder; b and c joined as mirrors.
    setup_register(&a, &b, DeviceRole::Mirror, DeviceKind::Terminal);
    setup_register(&a, &c, DeviceRole::Mirror, DeviceKind::Terminal);
    for x in [&b, &c, &ph] {
        setup_register(x, &a, DeviceRole::Holder, DeviceKind::Terminal);
    }
    // Every device knows the others (the ring's table travels by anchor and ops; the roles come from the ops).
    setup_register(&b, &c, DeviceRole::Mirror, DeviceKind::Terminal);
    setup_register(&c, &b, DeviceRole::Mirror, DeviceKind::Terminal);
    setup_register(&ph, &b, DeviceRole::Mirror, DeviceKind::Terminal);
    setup_register(&ph, &c, DeviceRole::Mirror, DeviceKind::Terminal);
    setup_register(&b, &ph, DeviceRole::Holder, DeviceKind::Phone);
    setup_register(&c, &ph, DeviceRole::Holder, DeviceKind::Phone);
    // b and c joined as mirrors: that is what their own tables say about themselves too.
    setup_register(&b, &b, DeviceRole::Mirror, DeviceKind::Terminal);
    setup_register(&c, &c, DeviceRole::Mirror, DeviceKind::Terminal);
    setup_register(&ph, &ph, DeviceRole::Holder, DeviceKind::Phone);
    // b and c were admitted to the ring as mirrors by ops (what a hub's admission leaves in every log): a position in
    // the log from which a LATER promotion must not be read backwards.
    for who in [&b, &c] {
        append(
            &a,
            OpBody::DeviceAdd {
                device: who.id.public(),
                name: "joiner".into(),
                role: DeviceRole::Mirror,
                attendance: Attendance::Attended,
                kind: DeviceKind::Terminal,
                verified_by: VerifiedBy::Sas,
            },
        );
    }
    // The phone is enrolled on a (what `sync-phone-enroll` does) and a proposes its admission.
    setup_register(&a, &ph, DeviceRole::Holder, DeviceKind::Phone);
    let add = append(
        &a,
        OpBody::DeviceAdd {
            device: ph.id.public(),
            name: "phone".into(),
            role: DeviceRole::Holder,
            attendance: Attendance::Attended,
            kind: DeviceKind::Phone,
            verified_by: VerifiedBy::Sas,
        },
    );
    pull(&a, &ph);
    append(&ph, red_phone_sign_body(&add));
    pull(&ph, &a);
    let now = add.timestamp + 60;
    let st = red_status(&a.store, &add, now).unwrap();
    assert!(st.complete, "the first phone of a ring: {st:?}");

    // `sync-role b holder`: the phone exists now, so its proof is needed; b signs for itself; no factor exists.
    let role = append(
        &a,
        OpBody::DeviceRoleChange {
            device_id: b.did.clone(),
            to: DeviceRole::Holder,
        },
    );
    let st = red_status(&a.store, &role, now).unwrap();
    assert!(
        st.blocked.is_none(),
        "no reshare is needed without a Quorum credential: {st:?}"
    );
    assert!(
        st.missing.contains(&RedProof::Phone) && st.missing.contains(&RedProof::SecondDevice),
        "{st:?}"
    );
    assert!(
        !st.missing.iter().any(|m| matches!(m, RedProof::Factor(_))),
        "no factor can exist at setup: {st:?}"
    );
    pull(&a, &b);
    append(&b, red_approve_body(&role));
    pull(&b, &a);
    pull(&a, &ph);
    append(&ph, red_phone_sign_body(&role));
    pull(&ph, &a);
    let st = red_status(&a.store, &role, now).unwrap();
    assert!(st.complete, "{st:?}");
    assert!(
        st.note.as_deref().is_some_and(|n| n.contains("ring setup")),
        "what was waived is said, not silent: {st:?}"
    );

    // The ring has a phone and two terminal holders in force: setup is over, for good.
    let third = append(
        &a,
        OpBody::DeviceRoleChange {
            device_id: c.did.clone(),
            to: DeviceRole::Holder,
        },
    );
    pull(&a, &c);
    append(&c, red_approve_body(&third));
    pull(&c, &a);
    let st = red_status(&a.store, &third, now).unwrap();
    assert!(
        !st.complete,
        "the setup exemption is refused after setup: {st:?}"
    );
    assert!(st.missing.contains(&RedProof::Phone), "{st:?}");
    assert!(
        st.missing.iter().any(|m| matches!(m, RedProof::Factor(_))),
        "{st:?}"
    );
    assert!(st.note.is_none(), "{st:?}");

    // A later Red op (the paper registration, a bootstrap: phone and a second holder) completes, and nothing
    // that was valid at ring setup is judged again against the ring as it is now: on every device view the
    // phone and b stay holders, c (whose promotion is not complete) stays a mirror, proposer included.
    let reg = append(
        &a,
        crate::sync::quorum::paper_registration(&bip39_encode(&[0x37; 32])).unwrap(),
    );
    pull(&a, &ph);
    append(&ph, red_phone_sign_body(&reg));
    pull(&a, &b);
    append(&b, red_approve_body(&reg));
    for x in [&ph, &b, &c] {
        pull(x, &a);
    }
    let st = red_status(&a.store, &reg, now).unwrap();
    assert!(st.complete, "{st:?}");
    for x in [&b, &c, &ph] {
        pull(&a, x);
        pull(&ph, x);
        pull(&b, x);
    }
    // Each device settles what completed (what every sync round does).
    for x in [&a, &b, &c, &ph] {
        settle_red_ops(&x.store, now).unwrap();
    }
    let roles = |on: &Dev| -> Vec<(String, DeviceRole)> {
        let mut v: Vec<_> = effective_devices(&on.store, None)
            .unwrap()
            .into_iter()
            .filter(|d| [&ph.did, &b.did, &c.did].contains(&&d.id))
            .map(|d| (d.id, d.role))
            .collect();
        v.sort_by(|x, y| x.0.cmp(&y.0));
        v
    };
    let mut want = vec![
        (ph.did.clone(), DeviceRole::Holder),
        (b.did.clone(), DeviceRole::Holder),
        (c.did.clone(), DeviceRole::Mirror),
    ];
    want.sort_by(|x, y| x.0.cmp(&y.0));
    for (name, on) in [("a", &a), ("b", &b), ("c", &c), ("phone", &ph)] {
        assert_eq!(
            roles(on),
            want,
            "the view of {name}: role {:?} third {:?} lam {} {}",
            red_status(&on.store, &role, now),
            red_status(&on.store, &third, now),
            role.lamport,
            third.lamport
        );
    }

    // The same ops, fed to a fresh device in any arrival order, give the same roles (no verdict depends on the
    // order or on which device proposed what).
    let chains: Vec<(&str, Vec<Op>)> = [("a", &a), ("b", &b), ("c", &c), ("phone", &ph)]
        .into_iter()
        .map(|(n, d)| (n, d.store.sync_chain_tail(&d.did, 0).unwrap()))
        .collect();
    let orders: [&[&str]; 4] = [
        &["a", "b", "c", "phone"],
        &["phone", "c", "b", "a"],
        &["b", "phone", "a", "c"],
        &["c", "a", "phone", "b"],
    ];
    for order in orders {
        let v = Dev::new();
        crate::sync::ceremony::test_force(&v.store, false);
        // What a device that joined through an anchor holds: the founder as holder, the joiners as mirrors.
        setup_register(&v, &a, DeviceRole::Holder, DeviceKind::Terminal);
        setup_register(&v, &b, DeviceRole::Mirror, DeviceKind::Terminal);
        setup_register(&v, &c, DeviceRole::Mirror, DeviceKind::Terminal);
        for who in order {
            for op in &chains.iter().find(|(n, _)| n == who).unwrap().1 {
                v.store.sync_ingest(op.clone()).unwrap();
            }
        }
        for _ in 0..4 {
            settle_red_ops(&v.store, now).unwrap();
        }
        if roles(&v) != want {
            let mut dbg = String::new();
            for (kind, who) in [
                ("add", &add),
                ("role", &role),
                ("reg", &reg),
                ("third", &third),
            ] {
                dbg += &format!(
                    "{kind}: {:?} state {:?}\n",
                    red_status(&v.store, who, now),
                    v.store
                        .sync_pending()
                        .unwrap()
                        .iter()
                        .any(|p| p.hash() == who.hash())
                );
            }
            dbg += &format!(
                "ph={} b={} c={} a={} lam add={} role={} reg={} third={}\n{:?}",
                ph.did,
                b.did,
                c.did,
                a.did,
                add.lamport,
                role.lamport,
                reg.lamport,
                third.lamport,
                effective_devices(&v.store, None).map(|r| r
                    .into_iter()
                    .map(|d| (d.id, d.role, d.kind))
                    .collect::<Vec<_>>())
            );
            let rows: Vec<(String, String, String, i64, i64)> = v.store.with_conn(|conn| {
                let mut q = conn.prepare("SELECT kind, state, coalesce(reason,''), lamport, seq FROM sync_ops WHERE device_id = ?1 ORDER BY seq")?;
                let r = q.query_map([&ph.did], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(r)
            }).unwrap();
            panic!("a fresh device fed in the order {order:?}\n{dbg}\nphone ops {rows:?}");
        }
    }
}

// ------------------------------------------------------------- verdicts are a pure function of the log (D0)

/// Three terminals and the phone in Quorum mode with the paper registered, both by finished ceremonies.
fn quorum_ring() -> Vec<Dev> {
    use crate::sync::quorum::paper_registration;
    use crate::sync::quorum::tests::{complete, ring_phone};
    let d = ring_phone(3);
    let phrase = bip39_encode(&[0x77; 32]);
    let reg = append(&d[0], paper_registration(&phrase).unwrap());
    complete(&d, &reg, 1, None);
    let mc = append(
        &d[0],
        OpBody::ModeChange {
            to: CredMode::Quorum,
            migrated_on: d[0].did.clone(),
            cosigner: None,
        },
    );
    complete(&d, &mc, 1, Some(&phrase));
    d
}

fn put_credential(x: &Dev, host: u8) -> Op {
    use crate::sync::quorum::tests::payload;
    crate::sync::cred_ops::cred_put(&x.store, &x.id, &[host; 16], &payload("pw")).unwrap()
}

fn forget_contacts(d: &[Dev]) {
    for x in d {
        x.store
            .with_conn(|c| {
                c.execute("UPDATE sync_devices SET last_seen = 0", [])?;
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn log_live_counts_only_devices_with_an_op_before_the_instant_and_inside_the_window() {
    let d = quorum_ring();
    sync(&d);
    let rows = effective_devices(&d[1].store, None).unwrap();
    let ids = |t: i64| -> Vec<String> {
        let mut v: Vec<String> = log_live(&d[1].store, rows.clone(), t)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        v.sort();
        v
    };
    let now = crate::store::now_ts();
    // Terminal 1 approved and terminal 0 proposed (both wrote ops); terminal 2 never wrote anything.
    let live = ids(now + 60);
    assert!(
        live.contains(&d[0].did) && live.contains(&d[1].did),
        "{live:?}"
    );
    assert!(
        !live.contains(&d[2].did),
        "a device with no op is not live: {live:?}"
    );
    // Before any op was written nobody is live; a window later nobody is.
    assert!(ids(0).is_empty());
    assert!(ids(now + LIVE_DEVICE_WINDOW_SECS + 3600).is_empty());
}

#[test]
fn a_promotion_verdict_follows_the_log_not_the_contact_clock() {
    let d = quorum_ring();
    put_credential(&d[0], 7);
    // Terminals 1 and 2 each wrote something: the log shows them alive. The contact clock (what this
    // device happened to hear when) says nobody was ever seen; it must change nothing.
    d[1].append_host();
    d[2].append_host();
    sync(&d);
    forget_contacts(&d);
    let promo = propose(
        &d,
        R,
        OpBody::DeviceRoleChange {
            device_id: d[2].did.clone(),
            to: DeviceRole::Holder,
        },
    );
    forget_contacts(&d);
    // Judged where the Quorum credential is applied (a Yellow credential waits for approval on a peer that merely pulled).
    let st = red_status(&d[0].store, &promo, promo.timestamp + 60).unwrap();
    assert_eq!(st.blocked, None, "the verdict read last_seen: {st:?}");
}

// ------------------------------------------------------------- the promotion block clears (F2)

#[test]
fn the_promotion_block_clears_when_no_quorum_credential_is_left() {
    use crate::sync::oplog::WrappedShare;
    let d = quorum_ring();
    let host = [9u8; 16];
    let first = put_credential(&d[0], 9);
    assert!(quorum_credentials_before(&d[0].store, u64::MAX).unwrap());
    // The same host is saved again with ONE wrapped share (a Vault shaped credential): the Quorum one is gone.
    let (ct, wraps) = d[0].store.sync_cred_ciphertext(&host).unwrap().unwrap();
    let later = append(
        &d[0],
        OpBody::CredUpdate {
            host_uid: host,
            ciphertext: ct,
            wrapped_shares: vec![WrappedShare {
                recipient: wraps[0].recipient.clone(),
                alg: wraps[0].alg,
                sealed: wraps[0].sealed.clone(),
            }],
        },
    );
    assert!(
        !quorum_credentials_before(&d[0].store, u64::MAX).unwrap(),
        "a ring that has no Quorum credential any more is still treated as having one"
    );
    // Judged at the op's own position: before the replacement the credential was a Quorum one.
    assert!(quorum_credentials_before(&d[0].store, later.lamport).unwrap());
    assert!(!quorum_credentials_before(&d[0].store, first.lamport).unwrap());
}
