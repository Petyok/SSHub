//! The recovery state machine: every state against every event, and the transitions that matter.

use super::*;

const T0: i64 = 1_000_000;
const NOW: i64 = T0 + 100;

/// The tests that read or set `SSHUB_E2E_SOLO_LOCK_SECS` never run side by side.
static SOLO_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The lock-shortening variable exists only in a build with `--features e2e-hooks`.
#[test]
fn the_solo_lock_env_hook_exists_only_in_the_e2e_build() {
    let _env = SOLO_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let paper = Authorization::Paper {
        signature_valid: true,
    };
    let mut r = Recovery::new();
    r.apply(Event::Propose(Proposal {
        solo: true,
        ..proposal(paper)
    }))
    .unwrap();
    r.apply(Event::Ack {
        by: id("req"),
        seen_at: T0 + 60,
        attended: true,
    })
    .unwrap();
    std::env::set_var("SSHUB_E2E_SOLO_LOCK_SECS", "3600");
    let v = r.verdict(T0 + 60 + 3600);
    std::env::remove_var("SSHUB_E2E_SOLO_LOCK_SECS");
    #[cfg(feature = "e2e-hooks")]
    assert_eq!(v, ReenrollVerdict::Ready);
    #[cfg(not(feature = "e2e-hooks"))]
    assert_eq!(
        v,
        ReenrollVerdict::NeedsMoreTime {
            ready_at: T0 + 60 + 14 * 86_400
        }
    );
}

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
        solo: false,
    }
}

/// Design 5.2: with no other attended computer the requester's own acknowledgement starts a lock of 14 days; the paper
/// is the authorization.
#[test]
fn a_solo_recovery_is_acknowledged_by_its_requester_and_locks_for_fourteen_days() {
    let _env = SOLO_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let paper = Authorization::Paper {
        signature_valid: true,
    };
    let solo = Proposal {
        solo: true,
        ..proposal(paper)
    };
    let mut r = Recovery::new();
    r.apply(Event::Propose(solo)).unwrap();
    r.apply(Event::Ack {
        by: id("req"),
        seen_at: T0 + 60,
        attended: true,
    })
    .unwrap();
    let ready_at = T0 + 60 + 14 * 86_400;
    assert_eq!(
        r.verdict(T0 + 60 + 8 * 86_400),
        ReenrollVerdict::NeedsMoreTime { ready_at }
    );
    assert_eq!(r.verdict(ready_at), ReenrollVerdict::Ready);
    // With another attended computer the requester never acknowledges its own recovery.
    let mut n = Recovery::new();
    n.apply(Event::Propose(proposal(paper))).unwrap();
    assert!(n
        .apply(Event::Ack {
            by: id("req"),
            seen_at: T0 + 60,
            attended: true
        })
        .is_err());
    // A solo recovery is authorized by the paper: approvals cannot exist.
    assert!(Recovery::new()
        .apply(Event::Propose(Proposal {
            solo: true,
            ..proposal(Authorization::Approvals)
        }))
        .is_err());
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
        // A has written something before: a device that wrote an op (stamped before the request) is required in the set.
        w.d[A]
            .store
            .sync_append_local_at(
                &w.d[A].id,
                OpBody::HostUpsert {
                    host_uid: [9; 16],
                    fields: crate::sync::oplog::HostFields {
                        name: Some("seen".into()),
                        ..Default::default()
                    },
                },
                crate::store::now_ts() - 100,
            )
            .unwrap();
        w.sync();
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
        // The review marks exactly the ops the recovery completes: the revocation of the lost phone and
        // the one admission, not the premature revocation or a second admission.
        let rows = w.d[OBS].store.sync_devices().unwrap();
        let marked = |op: &Op| {
            crate::app::sync_view::pending_view(&w.d[OBS].store, op, &rows, None)
                .summary
                .contains(crate::app::sync_view::COMPLETES_RECOVERY)
        };
        assert!(marked(&revoke) && marked(&add));
        assert!(!marked(&premature) && !marked(&add2));
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

    // e2e scenario 80: a device learned from an op has last_seen 0 until its first round. A recovery started before
    // that round omitted it, rightly; contact after the request must not make the recovery invalid.
    #[test]
    fn contact_after_the_request_does_not_invalidate_a_recovery_that_left_a_silent_device_out() {
        let w = world();
        let set_seen = |on: usize, of: usize, t: i64| {
            w.d[on]
                .store
                .with_conn(|c| {
                    c.execute(
                        "UPDATE sync_devices SET last_seen = ?2 WHERE device_id = ?1",
                        rusqlite::params![w.d[of].did, t],
                    )?;
                    Ok(())
                })
                .unwrap();
        };
        // B has never been heard from by R (never wrote an op, last_seen 0): not part of the liveness set.
        set_seen(R, B, 0);
        let op = w.reenroll(false, None, None);
        let now = crate::store::now_ts();
        w.prove(&op, false);
        // The first contact with B comes after the request.
        set_seen(R, B, now + 5);
        // (The observer's own table says B was fresh: it may judge differently. The requester must not.)
        let on_r = validate_phone_reenroll(&w.d[R].store, &op, now + 10);
        assert!(
            !matches!(on_r, Err(ref e) if e.to_string().contains("liveness")),
            "{on_r:?}"
        );
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
        // The clock of the lock is this device's first sight of the ack (a second or two after `now`): judge with a margin.
        let ready_at = now + RECOVERY_TIMELOCK_SECS + 120;
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

    // R3 crypto fix wired into the CLI: the recovery and the cancellation prove the third factor.
    fn hex_id(op: &Op) -> String {
        op.hash().iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn the_cli_recovery_reaches_ready_only_after_the_factor_and_lapses_after_three_days() {
        use crate::cli::sync_admin::{factor, recovery_start, FactorKind};
        use crate::sync::ceremony::{red_approve_body, red_phone_sign_body};
        use crate::sync::recovery::{ack_body, approve_body};
        let w = world();
        for x in &w.d {
            crate::sync::ceremony::test_force(&x.store, false);
        }
        let now = crate::store::now_ts();
        let paper_reg = crate::sync::quorum::ops_of(&w.d[R].store, "FactorRegister", None)
            .unwrap()
            .remove(0);
        append(&w.d[P], red_phone_sign_body(&paper_reg));
        append(&w.d[A], red_approve_body(&paper_reg));
        // Started from the CLI function WITHOUT the third factor: the report says what is missing.
        let r = recovery_start(&w.d[R].store, &w.d[R].id, now, None, None).unwrap();
        assert!(
            r.lines.iter().any(|l| l.contains("not proven yet")),
            "{:?}",
            r.lines
        );
        let op = crate::sync::quorum::ops_of(&w.d[R].store, "PhoneReenroll", None)
            .unwrap()
            .pop()
            .unwrap();
        append(&w.d[A], approve_body(&op).unwrap());
        append(&w.d[B], approve_body(&op).unwrap());
        append(&w.d[A], ack_body(&op, now).unwrap());
        w.sync();
        let later = now + 8 * 86_400;
        assert_eq!(
            w.verdict(&op, later).unwrap(),
            ReenrollVerdict::AwaitingFactor,
            "approvals, acknowledgement and the time lock are not enough"
        );
        // A wrong paper phrase proves nothing.
        let wrong = bip39_encode(&[0x55; 32]);
        factor(
            &w.d[R].store,
            &w.d[R].id,
            &hex_id(&op),
            FactorKind::Paper,
            &wrong,
            later,
        )
        .unwrap();
        w.sync();
        assert_eq!(
            w.verdict(&op, later).unwrap(),
            ReenrollVerdict::AwaitingFactor
        );
        // The right one, through `sync-factor <op-id>`, makes it Ready.
        let out = factor(
            &w.d[R].store,
            &w.d[R].id,
            &hex_id(&op),
            FactorKind::Paper,
            &w.phrase,
            later,
        )
        .unwrap();
        assert!(
            out.lines.iter().any(|l| l.contains("recovery")),
            "{:?}",
            out.lines
        );
        w.sync();
        assert_eq!(w.verdict(&op, later).unwrap(), ReenrollVerdict::Ready);
        // Nobody completed it: three days after it became ready it lapses.
        assert_eq!(
            w.verdict(&op, now + 11 * 86_400).unwrap(),
            ReenrollVerdict::Lapsed
        );
    }

    #[test]
    fn a_cancellation_from_a_device_needs_a_proven_factor() {
        use crate::cli::sync_admin::{recovery_cancel, recovery_start, FactorKind};
        let w = world();
        let now = crate::store::now_ts();
        recovery_start(&w.d[R].store, &w.d[R].id, now, None, Some(&w.phrase)).unwrap();
        let op = crate::sync::quorum::ops_of(&w.d[R].store, "PhoneReenroll", None)
            .unwrap()
            .pop()
            .unwrap();
        w.sync();
        // No secret: refused, nothing written.
        let before = crate::sync::quorum::ops_of(&w.d[A].store, "PhoneReenrollCancel", None)
            .unwrap()
            .len();
        let e = recovery_cancel(
            &w.d[A].store,
            &w.d[A].id,
            &hex_id(&op),
            FactorKind::Paper,
            " ",
            now,
        )
        .unwrap_err();
        assert!(
            e.to_string().contains("paper phrase or the emailed code"),
            "{e}"
        );
        assert_eq!(
            crate::sync::quorum::ops_of(&w.d[A].store, "PhoneReenrollCancel", None)
                .unwrap()
                .len(),
            before
        );
        // A wrong phrase: the cancellation is written but does not take effect, and the report says so.
        let wrong = bip39_encode(&[0x55; 32]);
        let r = recovery_cancel(
            &w.d[A].store,
            &w.d[A].id,
            &hex_id(&op),
            FactorKind::Paper,
            &wrong,
            now,
        )
        .unwrap();
        assert!(r.lines[0].contains("did not take effect"), "{:?}", r.lines);
        w.sync();
        assert_ne!(
            validate_phone_reenroll(&w.d[OBS].store, &op, now).unwrap(),
            ReenrollVerdict::CancelledByFactor
        );
        // The right phrase cancels it.
        let r = recovery_cancel(
            &w.d[A].store,
            &w.d[A].id,
            &hex_id(&op),
            FactorKind::Paper,
            &w.phrase,
            now,
        )
        .unwrap();
        assert!(
            r.lines[0].starts_with("cancelled recovery"),
            "{:?}",
            r.lines
        );
        w.sync();
        assert_eq!(
            validate_phone_reenroll(&w.d[OBS].store, &op, now).unwrap(),
            ReenrollVerdict::CancelledByFactor
        );
    }

    fn phone_swap(w: &World, by: usize, now: i64) -> Dev {
        let new = Dev::new();
        append(
            &w.d[by],
            OpBody::DeviceRevoke {
                device_id: w.d[P].did.clone(),
            },
        );
        let _ = now;
        append(
            &w.d[by],
            OpBody::DeviceAdd {
                device: new.id.public(),
                name: "new phone".into(),
                role: DeviceRole::Holder,
                attendance: Attendance::Attended,
                kind: DeviceKind::Phone,
                verified_by: VerifiedBy::Sas,
            },
        );
        w.sync();
        new
    }

    // H1 (crypto): a cancellation by the phone that was live stays a cancellation after the ring swaps its phone.
    #[test]
    fn cancelled_recovery_stays_cancelled_after_a_phone_swap() {
        let w = world();
        let op = w.reenroll(false, None, None);
        let now = op.timestamp + 60;
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
            w.verdict(&op, now).unwrap(),
            ReenrollVerdict::CancelledByPhone
        );
        phone_swap(&w, A, now);
        assert_eq!(
            w.verdict(&op, now).unwrap(),
            ReenrollVerdict::CancelledByPhone,
            "the cancel was judged against today's phone and dropped"
        );
    }

    // H1 (crypto): a recovery that was started against the old phone cannot remove the phone that replaced it.
    #[test]
    fn recovery_started_before_a_phone_swap_cannot_revoke_the_new_phone() {
        use crate::sync::recovery::{ack_body, approve_body, ready_recovery_for, start_body};
        let w = world();
        let now = crate::store::now_ts();
        let long_ago = now - 9 * 86_400;
        let at =
            |d: &Dev, body: OpBody, t: i64| d.store.sync_append_local_at(&d.id, body, t).unwrap();
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
        at(
            &w.d[R],
            crate::sync::ceremony::red_factor_paper(&recover, &op),
            long_ago + 30,
        );
        at(&w.d[A], approve_body(&op).unwrap(), long_ago + 60);
        at(&w.d[B], approve_body(&op).unwrap(), long_ago + 61);
        at(
            &w.d[A],
            ack_body(&op, long_ago + 120).unwrap(),
            long_ago + 120,
        );
        w.sync();
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
        assert_eq!(w.verdict(&op, now).unwrap(), ReenrollVerdict::Ready);
        // Another path replaces the phone (a second recovery, a handover); then the old recovery's requester tries to
        // remove the phone that is live now.
        let new = phone_swap(&w, A, now);
        let revoke_new = at(
            &w.d[R],
            OpBody::DeviceRevoke {
                device_id: new.did.clone(),
            },
            now + 2,
        );
        w.sync();
        assert!(
            !ready_recovery_for(&w.d[OBS].store, &revoke_new, now + 2).unwrap(),
            "a recovery started against the old phone stood in for the ceremony that removes the new one"
        );
    }

    /// Only `R` stays an attended terminal: the others run on their own.
    fn solo_world() -> World {
        let w = world();
        for x in &w.d {
            paper_setup(&x.store, &w.phrase).unwrap();
        }
        for x in &w.d {
            x.store
                .with_conn(|c| {
                    c.execute(
                        "UPDATE sync_devices SET attendance = 'unattended' WHERE device_id IN (?1, ?2, ?3)",
                        rusqlite::params![w.d[A].did, w.d[B].did, w.d[OBS].did],
                    )?;
                    Ok(())
                })
                .unwrap();
        }
        w
    }

    fn settle_received_at(w: &World) {
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
    }

    // Design 5.2: a ring of one terminal and the phone: the paper authorizes and stands for the third factor too, the
    // requester's own acknowledgement starts a 14 day lock.
    #[test]
    fn recovery_in_a_one_terminal_ring_completes_after_the_lock() {
        let _env = super::SOLO_ENV.lock().unwrap_or_else(|e| e.into_inner());
        use crate::sync::recovery::{ack_body, is_solo, start_body};
        let w = solo_world();
        assert!(is_solo(&w.d[OBS].store, &w.d[R].did).unwrap());
        let now = crate::store::now_ts();
        let started = now - 15 * 86_400;
        let at =
            |d: &Dev, body: OpBody, t: i64| d.store.sync_append_local_at(&d.id, body, t).unwrap();
        let recover = paper_derive(&w.phrase, PaperDomain::Recover).unwrap();
        let start = start_body(
            &w.d[R].store,
            &w.d[R].id,
            started,
            Some(&recover),
            ThirdFactor::Paper,
        )
        .unwrap();
        let op = at(&w.d[R], start, started);
        at(
            &w.d[R],
            crate::sync::ceremony::red_factor_paper(&recover, &op),
            started + 30,
        );
        at(&w.d[R], ack_body(&op, started + 60).unwrap(), started + 60);
        w.sync();
        settle_received_at(&w);
        // The ordinary 7 day lock would be over eight days in; the solo lock is not.
        assert_eq!(
            validate_phone_reenroll(&w.d[OBS].store, &op, started + 60 + 8 * 86_400).unwrap(),
            ReenrollVerdict::NeedsMoreTime {
                ready_at: started + 60 + 14 * 86_400
            }
        );
        assert_eq!(
            validate_phone_reenroll(&w.d[OBS].store, &op, now).unwrap(),
            ReenrollVerdict::Ready
        );
    }

    #[test]
    fn solo_recovery_is_cancelled_by_the_live_phone() {
        use crate::sync::recovery::{ack_body, start_body};
        let w = solo_world();
        let now = crate::store::now_ts();
        let started = now - 15 * 86_400;
        let at =
            |d: &Dev, body: OpBody, t: i64| d.store.sync_append_local_at(&d.id, body, t).unwrap();
        let recover = paper_derive(&w.phrase, PaperDomain::Recover).unwrap();
        let start = start_body(
            &w.d[R].store,
            &w.d[R].id,
            started,
            Some(&recover),
            ThirdFactor::Paper,
        )
        .unwrap();
        let op = at(&w.d[R], start, started);
        at(
            &w.d[R],
            crate::sync::ceremony::red_factor_paper(&recover, &op),
            started + 30,
        );
        at(&w.d[R], ack_body(&op, started + 60).unwrap(), started + 60);
        at(
            &w.d[P],
            OpBody::PhoneReenrollCancel {
                reenroll_hash: op.hash(),
                by_phone: true,
                third_factor: None,
            },
            started + 120,
        );
        w.sync();
        settle_received_at(&w);
        assert_eq!(
            validate_phone_reenroll(&w.d[OBS].store, &op, now).unwrap(),
            ReenrollVerdict::CancelledByPhone
        );
    }

    #[test]
    fn an_open_recovery_alerts_the_other_attended_terminals_and_a_cancelled_one_does_not() {
        use crate::sync::recovery::{open_for_alert, open_for_alert_for};
        let w = world();
        // A device records itself at enrollment; the fixture does not for the terminals.
        w.d[A]
            .store
            .sync_register_device(
                &w.d[A].id.public(),
                "self",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
        let op = w.reenroll(false, None, None);
        let now = op.timestamp + 60;
        let alerts = open_for_alert_for(&w.d[A].store, &w.d[A].did, now).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].hash(), op.hash());
        assert_eq!(
            open_for_alert(&w.d[A].store, &w.d[A].id, now)
                .unwrap()
                .len(),
            1
        );
        // Not for the requester itself, nor for a device nobody sits at.
        assert!(open_for_alert_for(&w.d[A].store, &w.d[R].did, now)
            .unwrap()
            .is_empty());
        assert!(open_for_alert_for(&w.d[A].store, &w.d[H].did, now)
            .unwrap()
            .is_empty());
        append(
            &w.d[P],
            OpBody::PhoneReenrollCancel {
                reenroll_hash: op.hash(),
                by_phone: true,
                third_factor: None,
            },
        );
        w.sync();
        assert!(open_for_alert_for(&w.d[A].store, &w.d[A].did, now)
            .unwrap()
            .is_empty());
    }

    // H1 rule 4: a handover in force after a recovery started leaves that recovery superseded, and nobody is alerted to it.
    #[test]
    fn superseded_recovery_is_not_alerted() {
        use crate::sync::recovery::open_for_alert_for;
        let w = world();
        w.d[A]
            .store
            .sync_register_device(
                &w.d[A].id.public(),
                "self",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
        let op = w.reenroll(false, None, None);
        let now = op.timestamp + 60;
        assert_eq!(open_for_alert_for(&w.d[A].store, &w.d[A].did, now).unwrap().len(), 1);
        append(
            &w.d[P],
            OpBody::PhoneHandover {
                new_phone: Dev::new().id.public(),
                name: "next phone".into(),
            },
        );
        w.sync();
        assert_eq!(w.verdict(&op, now).unwrap(), ReenrollVerdict::Superseded);
        assert!(open_for_alert_for(&w.d[A].store, &w.d[A].did, now).unwrap().is_empty());
    }
}
