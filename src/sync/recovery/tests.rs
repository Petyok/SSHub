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
                now: T0 + 30 * 86_400,
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
                    now: T0 + 30 * 86_400,
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
    use crate::sync::oplog::{Attendance, DeviceKind, HeadAttest, Op, OpBody, ThirdFactor};
    use crate::sync::protocol::tests::Dev;
    use crate::sync::quorum::paper_setup;
    use crate::sync::quorum::tests::{append, deliver};

    /// 0 requester, 1 and 2 attended terminals, 3 the unattended hub, 4 the phone, 5 an observer
    /// that never writes and judges.
    struct World {
        d: Vec<Dev>,
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
        World { d }
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
            op
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
        append(
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
}
