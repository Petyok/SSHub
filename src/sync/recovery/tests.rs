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
            },
        ),
        (
            "ack-predating",
            Event::Ack {
                by: id("relay-peer"),
                seen_at: T0 - 10_000,
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
    // propose, invalid, approve, again, by-req, unattended, ack, ack-req, ack-pre, phone, phone-false, factor, no-factor, complete
    ("idle", "oeeeeeeeeeeeee"),
    ("await0", "eeooeeoeeoeoee"),
    ("await1", "eeooeeoeeoeoee"),
    ("await-ack", "eeooeeoeeoeoee"),
    ("timelock", "eeooeeoeeoeoee"),
    ("ready", "eeooeeoeeoeoeo"),
    ("cancelled-phone", "eeeeeeeeeeeeee"),
    ("cancelled-factor", "eeeeeeeeeeeeee"),
    ("completed", "eeeeeeeeeeeeee"),
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
