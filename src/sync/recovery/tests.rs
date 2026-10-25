//! The recovery state machine: every state against every event, and the transitions that matter.

use super::*;

const T0: i64 = 1_000_000;
const NOW: i64 = T0 + 100;

fn id(s: &str) -> DeviceId {
    s.into()
}

fn proposal(auth: Authorization) -> Proposal {
    Proposal {
        hash: [1; 32],
        requester: id("req"),
        timestamp: T0,
        initiated_at: T0 + 5,
        authorization: auth,
        third_factor_distinct: true,
        liveness_ok: true,
    }
}

fn approve(who: &str) -> Event {
    Event::Approve {
        by: id(who),
        attended: true,
    }
}

fn ack(at: i64) -> Event {
    Event::Ack {
        by: id("relay-peer"),
        seen_at: at,
        attended: true,
    }
}

const STATES: [&str; 9] = [
    "idle",
    "await0",
    "await1",
    "await-ack",
    "timelock",
    "ready",
    "cancelled-phone",
    "cancelled-factor",
    "completed",
];

fn state(name: &str) -> Recovery {
    let mut r = Recovery::new();
    let steps: Vec<Event> = match name {
        "idle" => vec![],
        "await0" => vec![Event::Propose(proposal(Authorization::Approvals))],
        "await1" => vec![
            Event::Propose(proposal(Authorization::Approvals)),
            approve("a"),
        ],
        "await-ack" => vec![
            Event::Propose(proposal(Authorization::Approvals)),
            approve("a"),
            approve("b"),
        ],
        "timelock" => vec![
            Event::Propose(proposal(Authorization::Approvals)),
            approve("a"),
            approve("b"),
            ack(T0 + 10),
        ],
        "ready" => vec![
            Event::Propose(proposal(Authorization::Approvals)),
            approve("a"),
            approve("b"),
            ack(T0 + 10),
        ],
        "cancelled-phone" => vec![
            Event::Propose(proposal(Authorization::Approvals)),
            Event::CancelByPhone { by_phone: true },
        ],
        "cancelled-factor" => vec![
            Event::Propose(proposal(Authorization::Approvals)),
            Event::CancelByDevice { with_factor: true },
        ],
        "completed" => vec![
            Event::Propose(proposal(Authorization::Approvals)),
            approve("a"),
            approve("b"),
            ack(T0 + 10),
            Event::Complete {
                now: T0 + RECOVERY_TIMELOCK_SECS + 3_600,
            },
        ],
        other => panic!("{other}"),
    };
    for e in steps {
        // "ready" and "completed" need the clock to have run out: the machine is told the time when asked.
        r.apply(e).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    r
}

/// Events in table order.
fn events() -> Vec<(&'static str, Event)> {
    let bad = Proposal {
        liveness_ok: false,
        ..proposal(Authorization::Approvals)
    };
    vec![
        (
            "propose",
            Event::Propose(proposal(Authorization::Approvals)),
        ),
        ("propose-invalid", Event::Propose(bad)),
        ("approve", approve("c")),
        ("approve-again", approve("a")),
        ("approve-by-requester", approve("req")),
        (
            "approve-unattended",
            Event::Approve {
                by: id("d"),
                attended: false,
            },
        ),
        ("ack", ack(T0 + 20)),
        (
            "ack-by-requester",
            Event::Ack {
                by: id("req"),
                seen_at: T0 + 20,
                attended: true,
            },
        ),
        (
            "ack-predating",
            Event::Ack {
                by: id("relay-peer"),
                seen_at: T0 - 10_000,
                attended: true,
            },
        ),
        (
            "ack-unattended",
            Event::Ack {
                by: id("hub"),
                seen_at: T0 + 20,
                attended: false,
            },
        ),
        ("cancel-by-phone", Event::CancelByPhone { by_phone: true }),
        (
            "cancel-claimed-by-phone",
            Event::CancelByPhone { by_phone: false },
        ),
        (
            "cancel-with-factor",
            Event::CancelByDevice { with_factor: true },
        ),
        (
            "cancel-without-factor",
            Event::CancelByDevice { with_factor: false },
        ),
        ("complete", Event::Complete { now: NOW }),
    ]
}

/// `o` the event is accepted in that state, `e` it is refused. Columns follow `events()`.
const EXPECT: [(&str, &str); 9] = [
    // propose, invalid, approve, again, by-req, unattended, ack, ack-req, ack-pre, ack-unattended, phone, phone-false, factor, no-factor, complete
    ("idle", "oeeeeeeeeeeeeee"),
    ("await0", "eeooeeoeeeoeoee"),
    ("await1", "eeooeeoeeeoeoee"),
    ("await-ack", "eeooeeoeeeoeoee"),
    ("timelock", "eeooeeoeeeoeoee"),
    ("ready", "eeooeeoeeeoeoeo"),
    ("cancelled-phone", "eeeeeeeeeeoeeee"),
    ("cancelled-factor", "eeeeeeeeeeoeeee"),
    ("completed", "eeeeeeeeeeeeeee"),
];

#[test]
fn every_state_against_every_event() {
    let evs = events();
    for (name, row) in EXPECT {
        assert_eq!(row.len(), evs.len(), "{name}");
        for ((ev_name, ev), want) in evs.iter().zip(row.chars()) {
            let mut r = state(name);
            let ev = match (name, ev) {
                ("ready", Event::Complete { .. }) => Event::Complete {
                    now: T0 + RECOVERY_TIMELOCK_SECS + 3_600,
                },
                (_, e) => e.clone(),
            };
            let got = r.apply(ev);
            // "ready" differs from "timelock" only in the time the verdict is asked at, so the
            // completion event is judged with the late clock inside `apply`.
            assert_eq!(
                got.is_ok(),
                want == 'o',
                "state {name}, event {ev_name}: {got:?}"
            );
        }
    }
    assert_eq!(STATES.len(), EXPECT.len());
}

#[test]
fn the_verdict_follows_the_events_and_the_clock() {
    let mut r = state("await0");
    assert_eq!(
        r.verdict(NOW),
        ReenrollVerdict::AwaitingApproval { have: 0, need: 2 }
    );
    r.apply(approve("a")).unwrap();
    r.apply(approve("a")).unwrap();
    assert_eq!(
        r.verdict(NOW),
        ReenrollVerdict::AwaitingApproval { have: 1, need: 2 },
        "the same device twice is one approval"
    );
    r.apply(approve("b")).unwrap();
    assert_eq!(
        r.verdict(NOW),
        ReenrollVerdict::AwaitingAck,
        "authorized, but the clock does not run without an acknowledgement"
    );
    r.apply(ack(T0 + 50)).unwrap();
    r.apply(ack(T0 + 30)).unwrap();
    let ready_at = T0 + 30 + RECOVERY_TIMELOCK_SECS;
    assert_eq!(
        r.verdict(NOW),
        ReenrollVerdict::NeedsMoreTime { ready_at },
        "the earliest acknowledgement starts the clock"
    );
    assert_eq!(
        r.verdict(ready_at - 1),
        ReenrollVerdict::NeedsMoreTime { ready_at }
    );
    assert_eq!(r.verdict(ready_at), ReenrollVerdict::Ready);
    assert!(
        r.clone()
            .apply(Event::Complete { now: ready_at - 1 })
            .is_err(),
        "not before the clock ran out"
    );
    r.apply(Event::Complete { now: ready_at }).unwrap();
    assert_eq!(r.verdict(0), ReenrollVerdict::Completed);
}

#[test]
fn an_acknowledgement_before_the_approvals_still_counts_once_they_arrive() {
    let mut r = state("await0");
    r.apply(ack(T0 + 1)).unwrap();
    assert!(matches!(
        r.verdict(NOW),
        ReenrollVerdict::AwaitingApproval { .. }
    ));
    r.apply(approve("a")).unwrap();
    r.apply(approve("b")).unwrap();
    assert!(matches!(
        r.verdict(NOW),
        ReenrollVerdict::NeedsMoreTime { .. }
    ));
}

#[test]
fn paper_authorizes_alone_and_approvals_do_not_double_count() {
    let mut r = Recovery::new();
    r.apply(Event::Propose(proposal(Authorization::Paper {
        signature_valid: true,
    })))
    .unwrap();
    assert_eq!(r.verdict(NOW), ReenrollVerdict::AwaitingAck);
    assert!(
        r.apply(approve("a")).is_err(),
        "approvals are not counted next to the paper signature"
    );
    let mut bad = Recovery::new();
    assert!(bad
        .apply(Event::Propose(proposal(Authorization::Paper {
            signature_valid: false
        })))
        .is_err());
    let mut same_factor = Recovery::new();
    let p = Proposal {
        third_factor_distinct: false,
        ..proposal(Authorization::Paper {
            signature_valid: true,
        })
    };
    assert!(
        same_factor.apply(Event::Propose(p)).is_err(),
        "paper twice is one artifact"
    );
}

#[test]
fn a_backdated_proposal_and_a_replay_are_refused() {
    let mut r = Recovery::new();
    let old = Proposal {
        initiated_at: T0 - 10 * 86_400,
        ..proposal(Authorization::Approvals)
    };
    assert!(r.apply(Event::Propose(old)).is_err(), "backdated");
    r.apply(Event::Propose(proposal(Authorization::Approvals)))
        .unwrap();
    let again = Proposal {
        hash: [2; 32],
        ..proposal(Authorization::Approvals)
    };
    assert!(
        r.apply(Event::Propose(again)).is_err(),
        "a second PhoneReenroll, replayed or fresh, is refused"
    );
    assert!(
        replay([approve("a")]).is_err(),
        "an event before any proposal"
    );
}

#[test]
fn cancellation_wins_over_everything_and_a_live_phone_needs_no_factor() {
    for (ev, want) in [
        (
            Event::CancelByPhone { by_phone: true },
            ReenrollVerdict::CancelledByPhone,
        ),
        (
            Event::CancelByDevice { with_factor: true },
            ReenrollVerdict::CancelledByFactor,
        ),
    ] {
        let mut r = state("timelock");
        r.apply(ev).unwrap();
        assert_eq!(r.verdict(T0 + 100 * 86_400), want);
    }
    assert!(
        state("await1")
            .apply(Event::CancelByDevice { with_factor: false })
            .is_err(),
        "a compromised peer cannot cancel for free"
    );
}

// ------------------------------------------------------------------ over the log

mod over_the_log {
    use super::*;
    use crate::sync::crypto::{bip39_encode, paper_derive, PaperDomain};
    use crate::sync::oplog::{
        Attendance, DeviceKind, DeviceRole, HeadAttest, Op, OpBody, ThirdFactor, VerifiedBy,
    };
    use crate::sync::protocol::tests::Dev;
    use crate::sync::quorum::paper_setup;
    use crate::sync::quorum::tests::{append, deliver};

    /// 0 requester, 1 and 2 attended terminals, 3 the unattended hub, 4 the phone, 5 an observer
    /// that never writes and judges.
    struct World {
        d: Vec<Dev>,
        phrase: String,
        code: String,
    }

    const R: usize = 0;
    const A: usize = 1;
    const B: usize = 2;
    const H: usize = 3;
    const P: usize = 4;
    const OBS: usize = 5;

    fn world() -> World {
        let d: Vec<Dev> = (0..6).map(|_| Dev::new()).collect();
        for (i, x) in d.iter().enumerate() {
            for (j, y) in d.iter().enumerate() {
                if i == j {
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
        // A device records itself at enrollment; the hub knows it is unattended.
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
        // The ring registered both third factors (ceremonies forced on in these fixtures): the paper, and the
        // emailed code with the hub as the mailbox duty.
        let phrase = bip39_encode(&[0x36; 32]);
        let (email_body, code) = crate::sync::ceremony::factor_register_email(&d[H].did).unwrap();
        append(
            &d[R],
            crate::sync::quorum::paper_registration(&phrase).unwrap(),
        );
        append(&d[R], email_body);
        let w = World { d, phrase, code };
        w.sync();
        w
    }

    impl World {
        fn sync(&self) {
            for x in &self.d {
                for y in &self.d {
                    if x.did != y.did {
                        deliver(x, y);
                    }
                }
            }
        }

        fn reenroll(
            &self,
            via_paper: bool,
            paper_sig: Option<[u8; 64]>,
            drop_liveness_of: Option<usize>,
        ) -> Op {
            let now = crate::store::now_ts();
            let mut liveness =
                crate::sync::recovery::liveness_set(&self.d[R].store, &self.d[R].did, now).unwrap();
            if let Some(i) = drop_liveness_of {
                liveness.retain(|a| a.device_id != self.d[i].did);
            }
            let op = append(
                &self.d[R],
                OpBody::PhoneReenroll {
                    initiated_at: now,
                    signers: vec![],
                    via_paper,
                    third_factor: if via_paper {
                        ThirdFactor::EmailCode
                    } else {
                        ThirdFactor::Paper
                    },
                    liveness,
                    paper_sig,
                },
            );
            self.sync();
            self.prove(&op, via_paper);
            op
        }

        /// The requester proves the recovery's third factor: the emailed code when paper authorizes, else
        /// the paper.
        fn prove(&self, op: &Op, emailed: bool) {
            let body = if emailed {
                crate::sync::ceremony::red_factor_email(&self.d[R].store, &self.code, op).unwrap()
            } else {
                let recover = paper_derive(&self.phrase, PaperDomain::Recover).unwrap();
                crate::sync::ceremony::red_factor_paper(&recover, op)
            };
            append(&self.d[R], body);
            self.sync();
        }

        fn verdict(&self, op: &Op, now: i64) -> anyhow::Result<ReenrollVerdict> {
            validate_phone_reenroll(&self.d[OBS].store, op, now)
        }
    }

    #[test]
    fn approvals_acknowledgement_clock_and_cancellations_over_a_real_log() {
        let w = world();
        let op = w.reenroll(false, None, None);
        let now = op.timestamp + 60;
        assert_eq!(
            w.verdict(&op, now).unwrap(),
            ReenrollVerdict::AwaitingApproval { have: 0, need: 2 }
        );

        // The hub's approval and acknowledgement count for nothing.
        append(
            &w.d[H],
            OpBody::RecoveryApprove {
                reenroll_hash: op.hash(),
            },
        );
        append(
            &w.d[H],
            OpBody::RecoveryAck {
                reenroll_hash: op.hash(),
                seen_at: now,
            },
        );
        // One attended approval, then the requester's own (ignored), then a second attended one.
        append(
            &w.d[A],
            OpBody::RecoveryApprove {
                reenroll_hash: op.hash(),
            },
        );
        append(
            &w.d[R],
            OpBody::RecoveryApprove {
                reenroll_hash: op.hash(),
            },
        );
        w.sync();
        assert_eq!(
            w.verdict(&op, now).unwrap(),
            ReenrollVerdict::AwaitingApproval { have: 1, need: 2 }
        );
        append(
            &w.d[B],
            OpBody::RecoveryApprove {
                reenroll_hash: op.hash(),
            },
        );
        w.sync();
        assert_eq!(
            w.verdict(&op, now).unwrap(),
            ReenrollVerdict::AwaitingAck,
            "the hub's acknowledgement did not start the clock"
        );

        append(
            &w.d[A],
            OpBody::RecoveryAck {
                reenroll_hash: op.hash(),
                seen_at: now,
            },
        );
        w.sync();
        let ready_at = now + RECOVERY_TIMELOCK_SECS;
        assert_eq!(
            w.verdict(&op, now + 10).unwrap(),
            ReenrollVerdict::NeedsMoreTime { ready_at }
        );
        assert_eq!(w.verdict(&op, ready_at).unwrap(), ReenrollVerdict::Ready);

        // A device cancels without the factor: ignored. With it: cancelled. The phone cancels alone.
        append(
            &w.d[B],
            OpBody::PhoneReenrollCancel {
                reenroll_hash: op.hash(),
                by_phone: false,
                third_factor: None,
            },
        );
        w.sync();
        assert_eq!(w.verdict(&op, ready_at).unwrap(), ReenrollVerdict::Ready);
        let named = append(
            &w.d[B],
            OpBody::PhoneReenrollCancel {
                reenroll_hash: op.hash(),
                by_phone: false,
                third_factor: Some(ThirdFactor::Paper),
            },
        );
        w.sync();
        assert_eq!(
            w.verdict(&op, ready_at).unwrap(),
            ReenrollVerdict::Ready,
            "a factor that is only named cancels nothing (review round 3, R3-F2)"
        );
        let recover = paper_derive(&w.phrase, PaperDomain::Recover).unwrap();
        append(
            &w.d[B],
            crate::sync::ceremony::red_factor_paper(&recover, &named),
        );
        w.sync();
        assert_eq!(
            w.verdict(&op, ready_at).unwrap(),
            ReenrollVerdict::CancelledByFactor
        );
        append(
            &w.d[P],
            OpBody::PhoneReenrollCancel {
                reenroll_hash: op.hash(),
                by_phone: true,
                third_factor: None,
            },
        );
        w.sync();
        assert_eq!(
            w.verdict(&op, ready_at).unwrap(),
            ReenrollVerdict::CancelledByPhone
        );
    }

    #[test]
    fn a_replayed_proposal_a_short_liveness_set_and_the_same_factor_twice_are_refused() {
        let w = world();
        let first = w.reenroll(false, None, None);
        assert!(w.verdict(&first, first.timestamp).is_ok());
        // The same body again, in a new op: refused. The first stays valid.
        let again = append(&w.d[R], first.body.clone());
        w.sync();
        assert!(w
            .verdict(&again, again.timestamp)
            .unwrap_err()
            .to_string()
            .contains("replay"));
        assert!(w.verdict(&first, first.timestamp).is_ok());

        // A liveness set that leaves out a device the judge knows to be fresh.
        let short = w.reenroll(false, None, Some(A));
        assert!(w
            .verdict(&short, short.timestamp)
            .unwrap_err()
            .to_string()
            .contains("liveness"));
    }

    #[test]
    fn paper_authorizes_alone_with_a_signature_over_the_body_and_never_with_the_same_factor_twice()
    {
        let w = world();
        let phrase = bip39_encode(&[0x5a; 32]);
        for x in &w.d {
            paper_setup(&x.store, &phrase).unwrap();
        }
        let recover = paper_derive(&phrase, PaperDomain::Recover).unwrap();
        let now = crate::store::now_ts();
        let liveness: Vec<HeadAttest> =
            crate::sync::recovery::liveness_set(&w.d[R].store, &w.d[R].did, now).unwrap();
        let body =
            |sig: Option<[u8; 64]>, via_paper: bool, tf: ThirdFactor| OpBody::PhoneReenroll {
                initiated_at: now,
                signers: vec![],
                via_paper,
                third_factor: tf,
                liveness: liveness.clone(),
                paper_sig: sig,
            };
        let unsigned = body(None, true, ThirdFactor::EmailCode);
        let sig = paper_reenroll_sign(&recover, &w.d[R].did, &unsigned);
        let signed = append(&w.d[R], body(Some(sig), true, ThirdFactor::EmailCode));
        w.sync();
        assert_eq!(
            w.verdict(&signed, signed.timestamp).unwrap(),
            ReenrollVerdict::AwaitingAck,
            "paper authorizes by itself"
        );
        // Approvals are not counted next to a paper signature.
        append(
            &w.d[A],
            OpBody::RecoveryApprove {
                reenroll_hash: signed.hash(),
            },
        );
        append(
            &w.d[B],
            OpBody::RecoveryApprove {
                reenroll_hash: signed.hash(),
            },
        );
        w.sync();
        assert_eq!(
            w.verdict(&signed, signed.timestamp).unwrap(),
            ReenrollVerdict::AwaitingAck
        );

        // A signature made for another requester or another body does not verify.
        let mut other = body(None, true, ThirdFactor::EmailCode);
        if let OpBody::PhoneReenroll { initiated_at, .. } = &mut other {
            *initiated_at += 1;
        }
        let stolen = paper_reenroll_sign(&recover, &w.d[R].did, &other);
        let bad = append(&w.d[R], body(Some(stolen), true, ThirdFactor::EmailCode));
        w.sync();
        assert!(w.verdict(&bad, bad.timestamp).is_err());
        // Paper twice (authorization and third factor) is one artifact.
        let sig2 =
            paper_reenroll_sign(&recover, &w.d[R].did, &body(None, true, ThirdFactor::Paper));
        let twice = append(&w.d[R], body(Some(sig2), true, ThirdFactor::Paper));
        w.sync();
        assert!(w.verdict(&twice, twice.timestamp).is_err());
        // A paper signature on a recovery that claims no paper is refused outright.
        let confused = append(&w.d[R], body(Some(sig), false, ThirdFactor::Paper));
        w.sync();
        assert!(w.verdict(&confused, confused.timestamp).is_err());
        // The phone does not request its own replacement; an unattended device cannot request one.
        let by_phone = append(&w.d[P], body(None, false, ThirdFactor::Paper));
        let by_hub = append(&w.d[H], body(None, false, ThirdFactor::EmailCode));
        w.sync();
        assert!(w.verdict(&by_phone, by_phone.timestamp).is_err());
        assert!(w.verdict(&by_hub, by_hub.timestamp).is_err());
    }

    // crypto round 2, O1: a Ready recovery completes: the lost phone is revoked, the new one admitted, and
    // the new phone supplies the phone proof of the next Red ceremony.
    #[test]
    fn a_ready_recovery_ends_with_a_new_phone_that_can_sign() {
        use crate::sync::ceremony::{in_force, red_approve_body, red_phone_sign_body, red_status};
        use crate::sync::quorum::phone_device;
        use crate::sync::recovery::{ack_body, approve_body, completion_bodies, start_body};
        let w = world();
        for x in &w.d {
            crate::sync::ceremony::test_force(&x.store, false);
        }
        let now = crate::store::now_ts();
        let long_ago = now - 9 * 86_400;
        let at =
            |d: &Dev, body: OpBody, t: i64| d.store.sync_append_local_at(&d.id, body, t).unwrap();
        // With the ceremony really on, the paper registration of the fixture needs its own proofs to be in
        // force (the first factor of a ring: the phone and a second device, no factor yet).
        let paper_reg = crate::sync::quorum::ops_of(&w.d[R].store, "FactorRegister", None)
            .unwrap()
            .remove(0);
        at(&w.d[P], red_phone_sign_body(&paper_reg), now);
        at(&w.d[A], red_approve_body(&paper_reg), now);
        let recover = paper_derive(&w.phrase, PaperDomain::Recover).unwrap();
        let start = start_body(
            &w.d[R].store,
            &w.d[R].id,
            long_ago,
            None,
            ThirdFactor::Paper,
        )
        .unwrap();
        let op = at(&w.d[R], start, long_ago);
        // The third factor is PROVEN: the paper signs this recovery (R3-F1).
        at(
            &w.d[R],
            crate::sync::ceremony::red_factor_paper(&recover, &op),
            long_ago + 30,
        );
        // Too early: the requester cannot revoke the phone yet.
        let premature = at(
            &w.d[R],
            OpBody::DeviceRevoke {
                device_id: w.d[P].did.clone(),
            },
            long_ago + 5,
        );
        w.sync();
        assert!(!in_force(&w.d[OBS].store, &premature).unwrap());
        at(&w.d[A], approve_body(&op).unwrap(), long_ago + 60);
        at(&w.d[B], approve_body(&op).unwrap(), long_ago + 61);
        at(
            &w.d[A],
            ack_body(&op, long_ago + 120).unwrap(),
            long_ago + 120,
        );
        w.sync();
        // Everybody first saw all of it nine days ago (the clock of the time lock is each device's own).
        for x in &w.d {
            x.store
                .with_conn(|c| {
                    c.execute(
                        "UPDATE sync_ops SET received_at = timestamp WHERE timestamp < received_at",
                        [],
                    )?;
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(
            w.verdict(&op, now).unwrap(),
            ReenrollVerdict::Ready,
            "{:?}",
            red_status(&w.d[OBS].store, &paper_reg, now)
        );
        // Ready: a revocation stamped before the lock ended stays out (its clock is its own stamp), the one
        // issued now stands without the lost phone's signature, and the replacement is admitted once.
        assert!(!in_force(&w.d[OBS].store, &premature).unwrap());
        let new = Dev::new();
        crate::sync::ceremony::test_force(&new.store, false);
        let bodies =
            completion_bodies(&w.d[R].store, &op, now, &new.id.public(), "new phone").unwrap();
        let revoke = at(&w.d[R], bodies[0].clone(), now);
        let add = at(&w.d[R], bodies[1].clone(), now + 1);
        w.sync();
        assert!(
            in_force(&w.d[OBS].store, &revoke).unwrap(),
            "{:?}",
            red_status(&w.d[OBS].store, &revoke, now)
        );
        assert!(
            in_force(&w.d[OBS].store, &add).unwrap(),
            "{:?}",
            red_status(&w.d[OBS].store, &add, now)
        );
        assert_eq!(
            phone_device(&w.d[OBS].store).unwrap(),
            Some(new.did.clone())
        );
        // A second admission through the same recovery is refused.
        let again = Dev::new();
        let add2 = at(
            &w.d[R],
            OpBody::DeviceAdd {
                device: again.id.public(),
                name: "another".into(),
                role: DeviceRole::Holder,
                attendance: Attendance::Attended,
                kind: DeviceKind::Phone,
                verified_by: VerifiedBy::Sas,
            },
            now + 1,
        );
        w.sync();
        assert!(!in_force(&w.d[OBS].store, &add2).unwrap());
        // The new phone is the ring's phone: it signs the next Red ceremony.
        let reg = at(
            &w.d[R],
            OpBody::BulkExport {
                requester: w.d[R].did.clone(),
                window_secs: 60,
            },
            now + 2,
        );
        w.sync();
        for i in [R, A, B] {
            new.trust_with(&w.d[i], Attendance::Attended, DeviceKind::Terminal);
        }
        new.trust_with(&w.d[P], Attendance::Attended, DeviceKind::Phone);
        deliver(&w.d[R], &new);
        // The new phone has the log from the requester now, but the lock ran long before: its clock for the
        // recovery is what the requester's receipts say, as on the other devices.
        new.store
            .with_conn(|c| {
                c.execute(
                    "UPDATE sync_ops SET received_at = timestamp WHERE timestamp < received_at",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        append(&new, red_phone_sign_body(&reg));
        append(&w.d[A], red_approve_body(&reg));
        append(
            &w.d[R],
            crate::sync::ceremony::red_factor_paper(&recover, &reg),
        );
        w.sync();
        deliver(&new, &w.d[OBS]);
        deliver(&new, &w.d[A]);
        let st = red_status(&w.d[OBS].store, &reg, now + 3).unwrap();
        assert!(st.complete, "{st:?}");
    }

    #[test]
    fn a_mirror_cannot_approve_or_acknowledge_a_recovery() {
        let w = world();
        let m = Dev::new();
        for x in w.d.iter().chain(std::iter::once(&m)) {
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
        let op = w.reenroll(false, None, None);
        let now = op.timestamp + 60;
        append(
            &m,
            OpBody::RecoveryApprove {
                reenroll_hash: op.hash(),
            },
        );
        append(
            &m,
            OpBody::RecoveryAck {
                reenroll_hash: op.hash(),
                seen_at: now,
            },
        );
        for y in &w.d {
            deliver(&m, y);
        }
        assert_eq!(
            w.verdict(&op, now).unwrap(),
            ReenrollVerdict::AwaitingApproval { have: 0, need: 2 }
        );
        append(
            &w.d[A],
            OpBody::RecoveryApprove {
                reenroll_hash: op.hash(),
            },
        );
        append(
            &w.d[B],
            OpBody::RecoveryApprove {
                reenroll_hash: op.hash(),
            },
        );
        w.sync();
        for y in &w.d {
            deliver(&m, y);
        }
        assert_eq!(
            w.verdict(&op, now).unwrap(),
            ReenrollVerdict::AwaitingAck,
            "a mirror's acknowledgement did not start the clock"
        );
    }

    // review round 3, R3-F1 and R3-F3: the third factor is proven, and a Ready recovery lapses.
    #[test]
    fn a_recovery_is_ready_only_with_a_proven_third_factor_and_lapses_unused() {
        use crate::sync::recovery::RECOVERY_READY_WINDOW_SECS;
        let w = world();
        let now = crate::store::now_ts();
        let op = append(
            &w.d[R],
            OpBody::PhoneReenroll {
                initiated_at: now,
                signers: vec![],
                via_paper: false,
                // Named, never proven.
                third_factor: ThirdFactor::EmailCode,
                liveness: crate::sync::recovery::liveness_set(&w.d[R].store, &w.d[R].did, now)
                    .unwrap(),
                paper_sig: None,
            },
        );
        w.sync();
        for x in [A, B] {
            append(
                &w.d[x],
                OpBody::RecoveryApprove {
                    reenroll_hash: op.hash(),
                },
            );
        }
        append(
            &w.d[A],
            OpBody::RecoveryAck {
                reenroll_hash: op.hash(),
                seen_at: now,
            },
        );
        w.sync();
        let ready_at = now + RECOVERY_TIMELOCK_SECS;
        assert_eq!(
            w.verdict(&op, ready_at).unwrap(),
            ReenrollVerdict::AwaitingFactor
        );
        // The paper cannot stand in for a factor the op did not name, and the emailed code must be
        // proven by the registered key: here the requester proves the named one.
        w.prove(&op, true);
        assert_eq!(w.verdict(&op, ready_at).unwrap(), ReenrollVerdict::Ready);
        assert_eq!(
            w.verdict(&op, ready_at + RECOVERY_READY_WINDOW_SECS)
                .unwrap(),
            ReenrollVerdict::Lapsed
        );
    }

    #[test]
    fn the_builders_drive_a_recovery_from_start_to_completion() {
        use crate::sync::recovery::{
            ack_body, approve_body, cancel_body, clear_snapshots, completion_bodies, start_body,
            unacknowledged,
        };
        let w = world();
        let now = crate::store::now_ts();
        let op = append(
            &w.d[R],
            start_body(&w.d[R].store, &w.d[R].id, now, None, ThirdFactor::Paper).unwrap(),
        );
        w.sync();
        w.prove(&op, false);
        // The attended devices see it, the requester and the unattended hub do not owe an acknowledgement.
        assert_eq!(
            unacknowledged(&w.d[A].store, &w.d[A].id, now)
                .unwrap()
                .len(),
            1
        );
        assert!(unacknowledged(&w.d[R].store, &w.d[R].id, now)
            .unwrap()
            .is_empty());
        assert!(unacknowledged(&w.d[H].store, &w.d[H].id, now)
            .unwrap()
            .is_empty());
        append(&w.d[A], approve_body(&op).unwrap());
        append(&w.d[B], approve_body(&op).unwrap());
        let ack = append(&w.d[A], ack_body(&op, now).unwrap());
        w.sync();
        assert!(
            unacknowledged(&w.d[A].store, &w.d[A].id, now)
                .unwrap()
                .is_empty(),
            "acknowledged once"
        );
        // The clock starts when this device first saw the acknowledgement, never before what it claims.
        let seen = crate::sync::ceremony::received_at(&w.d[OBS].store, &ack).unwrap();
        let ready_at = seen.max(now) + RECOVERY_TIMELOCK_SECS;
        assert_eq!(
            w.verdict(&op, now + 5).unwrap(),
            ReenrollVerdict::NeedsMoreTime { ready_at }
        );

        // Completion only after the clock, and it revokes the old phone and admits the new one.
        let new_phone = Dev::new();
        assert!(completion_bodies(
            &w.d[OBS].store,
            &op,
            now + 5,
            &new_phone.id.public(),
            "new phone"
        )
        .is_err());
        let bodies = completion_bodies(
            &w.d[OBS].store,
            &op,
            ready_at,
            &new_phone.id.public(),
            "new phone",
        )
        .unwrap();
        assert!(
            matches!(&bodies[0], OpBody::DeviceRevoke { device_id } if *device_id == w.d[P].did)
        );
        assert!(matches!(
            &bodies[1],
            OpBody::DeviceAdd {
                kind: DeviceKind::Phone,
                ..
            }
        ));

        // Cancel builders: a device needs the factor.
        assert!(cancel_body(&op, false, None).is_err());
        assert!(
            cancel_body(&op, false, Some(ThirdFactor::Paper)).is_ok()
                && cancel_body(&op, true, None).is_ok()
        );

        crate::sync::quorum::meta_set(&w.d[A].store, "snapshot:aa", b"x").unwrap();
        assert_eq!(clear_snapshots(&w.d[A].store).unwrap(), 1);
    }

    #[test]
    fn screening_rejects_bad_proposals_at_once_and_leaves_good_ones_pending() {
        use crate::sync::screen::screen_pending;
        let w = world();
        let now = crate::store::now_ts();
        let good_body = crate::sync::recovery::start_body(
            &w.d[R].store,
            &w.d[R].id,
            now,
            None,
            ThirdFactor::Paper,
        )
        .unwrap();
        let mut backdated = good_body.clone();
        if let OpBody::PhoneReenroll { initiated_at, .. } = &mut backdated {
            *initiated_at -= 10 * 86_400;
        }
        let good = append(&w.d[R], good_body);
        let bad = append(&w.d[R], backdated);
        // The observer ingests without anyone approving: both wait in review.
        for op in w.d[R].store.sync_chain_tail(&w.d[R].did, 0).unwrap() {
            w.d[OBS].store.sync_ingest(op).unwrap();
        }
        assert_eq!(w.d[OBS].store.sync_pending().unwrap().len(), 2);
        let report = screen_pending(&w.d[OBS].store, now).unwrap();
        assert_eq!(report.rejected.len(), 1);
        assert_eq!(report.rejected[0].0, bad.hash());
        assert!(
            report.rejected[0].1.contains("initiated_at"),
            "{:?}",
            report.rejected
        );
        let left = w.d[OBS].store.sync_pending().unwrap();
        assert_eq!((left.len(), left[0].hash()), (1, good.hash()));
    }
}
