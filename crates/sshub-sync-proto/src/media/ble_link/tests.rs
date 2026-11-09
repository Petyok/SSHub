//! Link layer tests: messages, chunk arithmetic, full exchanges in memory (clean, lossy, duplicated),
//! the limits of C20 and every timer, with a virtual clock.

use super::*;

fn frame(len: usize, kind: u8, seed: u8) -> Vec<u8> {
    let mut f = b"SWR1".to_vec();
    f.extend_from_slice(&[0, 1, kind]);
    f.extend((0..len.saturating_sub(7)).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)));
    f.truncate(len.max(7));
    f
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn chance(&mut self, per_mille: u64) -> bool {
        self.next() % 1000 < per_mille
    }
}

/// Two links wired together with a virtual clock; `lose` decides per message (from, bytes) whether
/// it is dropped, `dup` whether it is delivered twice.
struct Pair {
    term: Link,
    phone: Link,
    now: Instant,
    t_events: Vec<Event>,
    p_events: Vec<Event>,
}

impl Pair {
    fn new(proposal: usize, notify_cap: usize) -> Pair {
        let now = Instant::now();
        Pair {
            term: Link::terminal(now, notify_cap),
            phone: Link::phone(now, proposal),
            now,
            t_events: Vec::new(),
            p_events: Vec::new(),
        }
    }

    /// Delivers everything in flight until quiet; true if any message moved.
    fn flush(
        &mut self,
        lose: &mut dyn FnMut(bool, &mut Vec<u8>) -> bool,
        dup: &mut dyn FnMut() -> bool,
    ) -> bool {
        let mut moved = false;
        loop {
            let from_p = self.phone.take_outbox();
            let from_t = self.term.take_outbox();
            if from_p.is_empty() && from_t.is_empty() {
                break;
            }
            moved = true;
            for mut m in from_p {
                if lose(true, &mut m) {
                    continue;
                }
                self.term.on_message(self.now, &m);
                // HELLO and HELLO_ACK are reliable GATT writes: a second one is a real restart
                if m[0] >= T_DATA && dup() {
                    self.term.on_message(self.now, &m);
                }
            }
            for mut m in from_t {
                if lose(false, &mut m) {
                    continue;
                }
                self.phone.on_message(self.now, &m);
                if m[0] >= T_DATA && dup() {
                    self.phone.on_message(self.now, &m);
                }
            }
        }
        self.t_events.extend(self.term.take_events());
        self.p_events.extend(self.phone.take_events());
        moved
    }

    /// Runs until `done` holds or the virtual clock passes `limit_secs`.
    fn run(
        &mut self,
        limit_secs: u64,
        lose: &mut dyn FnMut(bool, &mut Vec<u8>) -> bool,
        dup: &mut dyn FnMut() -> bool,
        done: &dyn Fn(&Pair) -> bool,
    ) {
        let end = self.now + Duration::from_secs(limit_secs);
        loop {
            self.flush(lose, dup);
            if done(self) || self.now >= end {
                return;
            }
            self.now += Duration::from_millis(500);
            self.term.tick(self.now);
            self.phone.tick(self.now);
        }
    }

    fn frames(events: &[Event]) -> Vec<Vec<u8>> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Frame(f) => Some(f.clone()),
                _ => None,
            })
            .collect()
    }
}

fn clean() -> impl FnMut(bool, &mut Vec<u8>) -> bool {
    |_, _| false
}
fn no_dup() -> impl FnMut() -> bool {
    || false
}

#[test]
fn crc32_matches_the_standard_check_value() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

#[test]
fn messages_round_trip_and_reject_malformed_input() {
    let msgs = [
        Msg::Hello {
            version: 1,
            chunk: 244,
        },
        Msg::HelloAck {
            version: 1,
            chunk: 185,
            window: 8,
        },
        Msg::Data {
            frame_id: 7,
            idx: 0,
            count: 3,
            head: Some((500, 0xdead_beef)),
            payload: vec![1, 2, 3],
        },
        Msg::Data {
            frame_id: 7,
            idx: 2,
            count: 3,
            head: None,
            payload: vec![9; 100],
        },
        Msg::Ack {
            frame_id: 7,
            next: 2,
        },
        Msg::Abort {
            frame_id: 7,
            reason: A_CRC,
        },
        Msg::Bye { reason: B_USER },
        Msg::Ping,
    ];
    for m in &msgs {
        assert_eq!(Msg::decode(&m.encode()).as_ref(), Some(m));
    }
    for bad in [
        vec![],
        vec![0x01, 1, 0],
        vec![0x01, 1, 0, 244, 0],
        vec![0x02, 1, 0, 20],
        vec![0x10, 0, 1, 0, 0, 0, 1],
        vec![0x10, 0, 1, 0, 1, 0, 1],
        vec![0x10, 0, 1, 0, 0, 0, 1, 0, 0, 0, 5, 0, 0, 0, 0],
        vec![0x11, 0, 1, 0],
        vec![0x12, 0, 1],
        vec![0x20],
        vec![0x22, 0],
        vec![0x7f, 1, 2, 3],
    ] {
        assert_eq!(Msg::decode(&bad), None, "{bad:?}");
    }
}

#[test]
fn the_contract_byte_layout_is_exact() {
    assert_eq!(
        Msg::Hello {
            version: 1,
            chunk: 0x00f4
        }
        .encode(),
        [0x01, 0x01, 0x00, 0xf4]
    );
    assert_eq!(
        Msg::HelloAck {
            version: 1,
            chunk: 0x00b9,
            window: 8
        }
        .encode(),
        [0x02, 0x01, 0x00, 0xb9, 0x08]
    );
    assert_eq!(
        Msg::Data {
            frame_id: 0x0102,
            idx: 0,
            count: 0x0003,
            head: Some((0x0a0b_0c0d, 0x1112_1314)),
            payload: vec![0xee]
        }
        .encode(),
        [
            0x10, 0x01, 0x02, 0x00, 0x00, 0x00, 0x03, 0x0a, 0x0b, 0x0c, 0x0d, 0x11, 0x12, 0x13,
            0x14, 0xee
        ]
    );
    assert_eq!(
        Msg::Data {
            frame_id: 1,
            idx: 2,
            count: 3,
            head: None,
            payload: vec![0xaa, 0xbb]
        }
        .encode(),
        [0x10, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0xaa, 0xbb]
    );
    assert_eq!(
        Msg::Ack {
            frame_id: 9,
            next: 4
        }
        .encode(),
        [0x11, 0x00, 0x09, 0x00, 0x04]
    );
    assert_eq!(
        Msg::Abort {
            frame_id: 9,
            reason: 2
        }
        .encode(),
        [0x12, 0x00, 0x09, 0x02]
    );
    assert_eq!(Msg::Bye { reason: 1 }.encode(), [0x20, 0x01]);
    assert_eq!(Msg::Ping.encode(), [0x22]);
    assert_eq!(
        info_bytes("7QK2"),
        [1, 0, 0x10, 0, 0, b'7', b'Q', b'K', b'2']
    );
    assert_eq!(local_name("7QK2"), "SSHub-7QK2");
}

#[test]
fn chunk_arithmetic_matches_the_contract() {
    // chunk 20: first chunk carries 5 payload bytes, the others 13
    assert_eq!(chunk_count(7, 20), 2);
    assert_eq!(chunk_count(5, 20), 1);
    assert_eq!(chunk_count(18, 20), 2);
    assert_eq!(chunk_count(19, 20), 3);
    // chunk 244: 229 then 237
    assert_eq!(chunk_count(229, 244), 1);
    assert_eq!(chunk_count(230, 244), 2);
    assert_eq!(chunk_count(229 + 237, 244), 2);
    assert_eq!(chunk_count(229 + 237 + 1, 244), 3);
    assert_eq!(
        chunk_count(BLE_MAX_FRAME_BYTES, 244),
        1 + (BLE_MAX_FRAME_BYTES - 229).div_ceil(237)
    );
    // at the default size a 1 MiB frame fits the u16 chunk counter, at the minimum size it does not
    assert!(chunk_count(BLE_MAX_FRAME_BYTES, BLE_CHUNK_DEFAULT) < usize::from(u16::MAX));
    assert!(chunk_count(BLE_MAX_FRAME_BYTES, BLE_CHUNK_MIN) > usize::from(u16::MAX));
}

#[test]
fn tags_and_names_leak_nothing() {
    for _ in 0..200 {
        let t = new_tag();
        assert_eq!(t.len(), 4);
        assert!(t.bytes().all(|b| CROCKFORD.contains(&b)));
        assert!(!t.contains(['I', 'L', 'O', 'U']));
        assert!(local_name(&t).len() <= 10);
    }
    let a: std::collections::HashSet<String> = (0..50).map(|_| new_tag()).collect();
    assert!(a.len() > 40, "tags are random per arming");
}

#[test]
fn frames_both_ways_at_several_message_sizes() {
    for (proposal, cap) in [
        (20, 244),
        (185, 244),
        (244, 244),
        (512, 244),
        (244, 100),
        (244, 23 - 3),
    ] {
        for len in [
            7usize, 8, 19, 20, 21, 229, 230, 466, 467, 1000, 4096, 100_000,
        ] {
            let mut p = Pair::new(proposal, cap);
            p.run(5, &mut clean(), &mut no_dup(), &|p| {
                p.term.is_open() && p.phone.is_open()
            });
            let chunk = proposal.min(244).min(cap).max(20);
            assert_eq!(p.t_events[0], Event::Opened { chunk });
            let (up, down) = (frame(len, 2, 1), frame(len + 3, 1, 2));
            p.phone.send_frame(p.now, up.clone()).unwrap();
            p.term.send_frame(p.now, down.clone()).unwrap();
            p.run(600, &mut clean(), &mut no_dup(), &|p| {
                Pair::frames(&p.t_events).len() == 1 && Pair::frames(&p.p_events).len() == 1
            });
            assert_eq!(
                Pair::frames(&p.t_events),
                vec![up],
                "len {len} chunk {chunk}"
            );
            assert_eq!(
                Pair::frames(&p.p_events),
                vec![down],
                "len {len} chunk {chunk}"
            );
            assert!(p.p_events.contains(&Event::Sent) && p.t_events.contains(&Event::Sent));
        }
    }
}

#[test]
fn the_terminal_pushes_its_frame_as_soon_as_the_handshake_ends() {
    let mut p = Pair::new(244, 244);
    // queued before the phone even said hello
    p.term.send_frame(p.now, frame(300, 1, 7)).unwrap();
    assert!(
        p.term.take_outbox().is_empty(),
        "nothing is sent before HELLO_ACK"
    );
    p.run(5, &mut clean(), &mut no_dup(), &|p| {
        !Pair::frames(&p.p_events).is_empty()
    });
    assert_eq!(Pair::frames(&p.p_events), vec![frame(300, 1, 7)]);
}

#[test]
fn a_newer_queued_frame_replaces_one_that_has_not_started_and_frames_follow_in_order() {
    let mut p = Pair::new(244, 244);
    p.run(5, &mut clean(), &mut no_dup(), &|p| {
        p.term.is_open() && p.phone.is_open()
    });
    let first = frame(3000, 1, 1);
    p.term.send_frame(p.now, first.clone()).unwrap();
    // `first` is in flight; `a` is replaced by `b` before it starts
    p.term.send_frame(p.now, frame(50, 5, 2)).unwrap();
    let b = frame(60, 9, 3);
    p.term.send_frame(p.now, b.clone()).unwrap();
    p.run(60, &mut clean(), &mut no_dup(), &|p| {
        Pair::frames(&p.p_events).len() >= 2
    });
    assert_eq!(Pair::frames(&p.p_events), vec![first, b]);
}

#[test]
fn a_lossy_link_still_delivers_exactly_once() {
    for seed in 1..=12u64 {
        let mut rng = Rng(seed * 2_654_435_761);
        let mut rng2 = Rng(seed * 40_503);
        let mut p = Pair::new(244, 244);
        let (up, down) = (frame(20_000, 6, 1), frame(9_000, 3, 2));
        p.phone.send_frame(p.now, up.clone()).unwrap();
        p.term.send_frame(p.now, down.clone()).unwrap();
        // 2% loss of DATA, ACK and PING messages both ways, 10% duplicates; the handshake travels
        // as reliable GATT writes and is not dropped
        p.run(
            900,
            &mut |_, m| m[0] >= T_DATA && rng.chance(20),
            &mut || rng2.chance(100),
            &|p| Pair::frames(&p.t_events).len() == 1 && Pair::frames(&p.p_events).len() == 1,
        );
        assert_eq!(Pair::frames(&p.t_events), vec![up], "seed {seed}");
        assert_eq!(Pair::frames(&p.p_events), vec![down], "seed {seed}");
    }
}

#[test]
fn a_lost_hello_ack_is_recovered_by_a_second_hello() {
    let mut p = Pair::new(244, 244);
    let mut dropped = false;
    p.run(
        3,
        &mut |from_phone, m| {
            let kill = !from_phone && m[0] == T_HELLO_ACK && !dropped;
            dropped |= kill;
            kill
        },
        &mut no_dup(),
        &|_| false,
    );
    assert!(p.term.is_open());
    assert!(!p.phone.is_open(), "the phone never heard the ack");
    // the page sends HELLO again: renegotiation is allowed at any time
    p.phone = Link::phone(p.now, 244);
    p.run(3, &mut clean(), &mut no_dup(), &|p| p.phone.is_open());
    assert!(p.phone.is_open() && p.term.is_open());
}

#[test]
fn frames_that_break_the_rules_are_refused_before_buffering() {
    let now = Instant::now();
    let mut t = Link::terminal(now, 244);
    t.on_message(
        now,
        &Msg::Hello {
            version: 1,
            chunk: 244,
        }
        .encode(),
    );
    t.take_outbox();
    let data0 = |len: u32, count: u16, payload: usize| {
        Msg::Data {
            frame_id: 1,
            idx: 0,
            count,
            head: Some((len, 0)),
            payload: vec![0; payload],
        }
        .encode()
    };
    // too large
    t.on_message(now, &data0(BLE_MAX_FRAME_BYTES as u32 + 1, 4400, 229));
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Abort {
            frame_id: 1,
            reason: A_TOO_LARGE
        })
    );
    // count does not match len
    t.on_message(now, &data0(1000, 2, 229));
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Abort {
            frame_id: 1,
            reason: A_TOO_LARGE
        })
    );
    // shorter than a header
    t.on_message(now, &data0(3, 1, 3));
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Abort {
            frame_id: 1,
            reason: A_TOO_LARGE
        })
    );
    // first payload of the wrong size
    t.on_message(now, &data0(1000, chunk_count(1000, 244) as u16, 100));
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Abort {
            frame_id: 1,
            reason: A_TOO_LARGE
        })
    );
    assert!(t.rx.is_none(), "nothing was kept");
    // a message above the agreed size
    let mut big = vec![T_DATA];
    big.extend_from_slice(&[0; 300]);
    t.on_message(now, &big);
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Abort {
            frame_id: 0,
            reason: A_TOO_LARGE
        })
    );
    // garbage is ignored, not answered
    t.on_message(now, &[0x55, 1, 2]);
    assert!(t.take_outbox().is_empty());
}

#[test]
fn a_frame_without_the_magic_or_with_an_unknown_kind_never_reaches_the_application() {
    for bad in [
        {
            let mut f = frame(100, 2, 1);
            f[0] = b'X';
            f
        },
        frame(100, 0, 1),
        frame(100, 11, 1),
    ] {
        let mut p = Pair::new(244, 244);
        p.run(2, &mut clean(), &mut no_dup(), &|p| p.term.is_open());
        // the page's side does not check on send; bypass send_frame's validation
        let crc = crc32(&bad);
        p.phone.tx = Some(Tx {
            id: 1,
            data: bad.clone(),
            crc,
            count: chunk_count(bad.len(), p.phone.chunk),
            acked: 0,
            sent: 0,
            deadline: p.now + Duration::from_secs(5),
            retries: 0,
            resent: false,
            max_sent: 0,
            started: p.now,
            attempt: 0,
        });
        p.phone.pump();
        p.run(5, &mut clean(), &mut no_dup(), &|_| false);
        assert!(Pair::frames(&p.t_events).is_empty());
        assert!(p.p_events.contains(&Event::SendFailed(A_BUSY)));
    }
}

#[test]
fn valid_frame_checks_magic_length_and_kind() {
    for k in 1..=12 {
        assert!(valid_frame(&frame(7, k, 0)), "kind {k}");
    }
    assert!(
        !valid_frame(b"nope") && !valid_frame(&frame(7, 0, 0)) && !valid_frame(&frame(7, 13, 0))
    );
    assert!(!valid_frame(&frame(7, 2, 0)[..6]));
}

/// Flips a payload bit of chunk `idx` of every frame the phone sends while `hit(frame_id)` holds.
fn corrupt_chunk(idx: u16, hit: impl Fn(u16) -> bool) -> impl FnMut(bool, &mut Vec<u8>) -> bool {
    move |from_phone, m| {
        if from_phone && m[0] == T_DATA && be16(&m[3..]) == idx && hit(be16(&m[1..])) {
            let n = m.len() - 1;
            m[n] ^= 0x01;
        }
        false
    }
}

#[test]
fn a_corrupted_chunk_fails_the_crc_and_the_sender_retries_under_a_new_id() {
    let mut p = Pair::new(244, 244);
    p.run(2, &mut clean(), &mut no_dup(), &|p| {
        p.term.is_open() && p.phone.is_open()
    });
    let f = frame(2000, 2, 9);
    p.phone.send_frame(p.now, f.clone()).unwrap();
    // the first attempt (frame id 1) arrives with a flipped bit, the retry (id 2) is clean
    p.run(
        60,
        &mut corrupt_chunk(2, |id| id == 1),
        &mut no_dup(),
        &|p| !Pair::frames(&p.t_events).is_empty(),
    );
    assert_eq!(Pair::frames(&p.t_events), vec![f]);
    assert_eq!(p.phone.next_id, 3, "one retry, under a new frame id");
    assert!(p.p_events.contains(&Event::Sent));
}

#[test]
fn a_frame_that_keeps_failing_the_crc_is_given_up_after_three_attempts() {
    let mut p = Pair::new(244, 244);
    p.run(2, &mut clean(), &mut no_dup(), &|p| {
        p.term.is_open() && p.phone.is_open()
    });
    p.phone.send_frame(p.now, frame(900, 2, 9)).unwrap();
    p.run(60, &mut corrupt_chunk(2, |_| true), &mut no_dup(), &|p| {
        p.p_events.iter().any(|e| matches!(e, Event::SendFailed(_)))
    });
    assert!(p.p_events.contains(&Event::SendFailed(A_CRC)));
    assert!(Pair::frames(&p.t_events).is_empty());
    assert_eq!(p.phone.next_id, 1 + u16::from(BLE_RETRIES));
    assert!(!p.term.is_closed(), "a bad frame does not end the session");
}

#[test]
fn a_version_mismatch_is_answered_with_abort_5_and_bye() {
    let now = Instant::now();
    let mut t = Link::terminal(now, 244);
    t.on_message(
        now,
        &Msg::Hello {
            version: 2,
            chunk: 244,
        }
        .encode(),
    );
    let out: Vec<Msg> = t
        .take_outbox()
        .iter()
        .filter_map(|m| Msg::decode(m))
        .collect();
    assert_eq!(
        out,
        vec![
            Msg::Abort {
                frame_id: 0,
                reason: A_VERSION
            },
            Msg::Bye { reason: B_ERROR }
        ]
    );
    assert!(t.is_closed());
    assert_eq!(t.take_events(), vec![Event::Closed(B_ERROR)]);
}

#[test]
fn nothing_but_hello_is_taken_before_the_handshake() {
    let now = Instant::now();
    let mut t = Link::terminal(now, 244);
    t.on_message(
        now,
        &Msg::Ack {
            frame_id: 1,
            next: 1,
        }
        .encode(),
    );
    t.on_message(
        now,
        &Msg::Data {
            frame_id: 1,
            idx: 0,
            count: 1,
            head: Some((7, 0)),
            payload: vec![0; 5],
        }
        .encode(),
    );
    assert!(t.take_outbox().is_empty() && t.take_events().is_empty());
    assert!(!t.is_open());
    assert!(t.send_frame(now, frame(7, 1, 0)).is_ok());
    assert!(t.take_outbox().is_empty(), "queued, not sent");
}

#[test]
fn send_frame_refuses_what_the_channel_may_not_carry() {
    let now = Instant::now();
    let mut t = Link::terminal(now, 244);
    assert!(t.send_frame(now, vec![1, 2, 3]).is_err());
    assert!(t
        .send_frame(now, frame(BLE_MAX_FRAME_BYTES + 1, 2, 0))
        .is_err());
    assert!(t.send_frame(now, frame(BLE_MAX_FRAME_BYTES, 2, 0)).is_ok());
    t.bye(now, B_USER);
    assert!(t.send_frame(now, frame(10, 2, 0)).is_err());
}

#[test]
fn hello_must_arrive_in_time() {
    let now = Instant::now();
    let mut t = Link::terminal(now, 244);
    t.tick(now + Duration::from_secs(BLE_HELLO_SECS));
    assert!(!t.is_closed());
    t.tick(now + Duration::from_secs(BLE_HELLO_SECS + 1));
    assert!(t.is_closed());
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Bye { reason: B_TIMEOUT })
    );
    assert_eq!(t.take_events(), vec![Event::Closed(B_TIMEOUT)]);
}

#[test]
fn idle_sessions_ping_and_then_time_out() {
    let mut p = Pair::new(244, 244);
    p.run(2, &mut clean(), &mut no_dup(), &|p| {
        p.term.is_open() && p.phone.is_open()
    });
    // quiet link: both sides ping every 30 s, so nobody times out
    let mut pings = 0;
    for _ in 0..400 {
        p.now += Duration::from_secs(1);
        p.term.tick(p.now);
        p.phone.tick(p.now);
        for m in p.term.take_outbox() {
            pings += usize::from(Msg::decode(&m) == Some(Msg::Ping));
            p.phone.on_message(p.now, &m);
        }
        for m in p.phone.take_outbox() {
            p.term.on_message(p.now, &m);
        }
    }
    assert!(pings >= 10, "pings: {pings}");
    assert!(!p.term.is_closed() && !p.phone.is_closed());
    // the other side goes silent: after BLE_IDLE_SECS the terminal ends the session
    for _ in 0..(BLE_IDLE_SECS + 2) {
        p.now += Duration::from_secs(1);
        p.term.tick(p.now);
    }
    assert!(p.term.is_closed());
    let evs = p.term.take_events();
    assert!(evs.contains(&Event::Closed(B_TIMEOUT)));
}

#[test]
fn an_unacknowledged_frame_is_retried_three_times_then_the_session_ends() {
    let now = Instant::now();
    let mut t = Link::terminal(now, 244);
    t.on_message(
        now,
        &Msg::Hello {
            version: 1,
            chunk: 244,
        }
        .encode(),
    );
    t.send_frame(now, frame(5000, 1, 1)).unwrap();
    t.take_outbox();
    t.take_events();
    let mut at = now;
    let mut windows = 0;
    for _ in 0..(BLE_RETRIES + 2) {
        at += Duration::from_secs(BLE_ACK_SECS);
        t.on_message(at, &Msg::Ping.encode());
        t.tick(at);
        let out = t.take_outbox();
        windows += usize::from(out.iter().any(|m| m[0] == T_DATA));
        if t.is_closed() {
            break;
        }
    }
    assert_eq!(windows, usize::from(BLE_RETRIES));
    assert!(t.is_closed());
    let evs = t.take_events();
    assert!(evs.contains(&Event::SendFailed(A_TIMEOUT)) && evs.contains(&Event::Closed(B_TIMEOUT)));
}

#[test]
fn a_frame_that_never_completes_is_dropped_at_the_frame_deadline() {
    let now = Instant::now();
    let mut t = Link::terminal(now, 244);
    t.on_message(
        now,
        &Msg::Hello {
            version: 1,
            chunk: 244,
        }
        .encode(),
    );
    t.take_outbox();
    let f = frame(5000, 2, 1);
    let first = Msg::Data {
        frame_id: 1,
        idx: 0,
        count: chunk_count(f.len(), 244) as u16,
        head: Some((f.len() as u32, crc32(&f))),
        payload: f[..229].to_vec(),
    };
    t.on_message(now, &first.encode());
    assert!(t.rx.is_some());
    // keep the link alive but never finish the frame
    let mut at = now;
    while at < now + Duration::from_secs(BLE_FRAME_SECS + 5) {
        at += Duration::from_secs(10);
        t.on_message(at, &Msg::Ping.encode());
        t.tick(at);
    }
    assert!(t.rx.is_none());
    assert!(t.take_outbox().iter().any(|m| Msg::decode(m)
        == Some(Msg::Abort {
            frame_id: 1,
            reason: A_TIMEOUT
        })));
    assert!(!t.is_closed());
}

#[test]
fn a_second_frame_while_one_is_in_progress_is_refused_busy() {
    let now = Instant::now();
    let mut t = Link::terminal(now, 244);
    t.on_message(
        now,
        &Msg::Hello {
            version: 1,
            chunk: 244,
        }
        .encode(),
    );
    t.take_outbox();
    let f = frame(5000, 2, 1);
    let m = |id: u16| Msg::Data {
        frame_id: id,
        idx: 0,
        count: chunk_count(f.len(), 244) as u16,
        head: Some((f.len() as u32, crc32(&f))),
        payload: f[..229].to_vec(),
    };
    t.on_message(now, &m(1).encode());
    t.take_outbox();
    t.on_message(now, &m(2).encode());
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Abort {
            frame_id: 2,
            reason: A_BUSY
        })
    );
    assert!(
        t.rx.as_ref().is_some_and(|r| r.id == 1),
        "the frame in progress is untouched"
    );
}

#[test]
fn gaps_and_duplicates_are_answered_with_where_the_receiver_is() {
    let now = Instant::now();
    let mut t = Link::terminal(now, 244);
    t.on_message(
        now,
        &Msg::Hello {
            version: 1,
            chunk: 244,
        }
        .encode(),
    );
    t.take_outbox();
    t.take_events(); // Opened
    let f = frame(2000, 2, 1);
    let count = chunk_count(f.len(), 244) as u16;
    let chunk = |idx: u16| {
        let (from, to) = if idx == 0 {
            (0, 229)
        } else {
            (
                229 + (usize::from(idx) - 1) * 237,
                229 + usize::from(idx) * 237,
            )
        };
        Msg::Data {
            frame_id: 1,
            idx,
            count,
            head: (idx == 0).then(|| (f.len() as u32, crc32(&f))),
            payload: f[from..to.min(f.len())].to_vec(),
        }
        .encode()
    };
    t.on_message(now, &chunk(0));
    t.on_message(now, &chunk(1));
    assert!(t.take_outbox().is_empty(), "no ACK until window/2 chunks");
    t.on_message(now, &chunk(1)); // duplicate
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Ack {
            frame_id: 1,
            next: 2
        })
    );
    t.on_message(now, &chunk(4)); // gap
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Ack {
            frame_id: 1,
            next: 2
        })
    );
    for i in 2..count {
        t.on_message(now, &chunk(i));
    }
    let out: Vec<Msg> = t
        .take_outbox()
        .iter()
        .filter_map(|m| Msg::decode(m))
        .collect();
    assert_eq!(
        out.last(),
        Some(&Msg::Ack {
            frame_id: 1,
            next: count
        })
    );
    assert_eq!(t.take_events(), vec![Event::Frame(f.clone())]);
    // the last chunk again (the ACK was lost): acknowledged again, delivered once
    t.on_message(now, &chunk(count - 1));
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Ack {
            frame_id: 1,
            next: count
        })
    );
    assert!(t.take_events().is_empty());
    t.on_message(now, &chunk(0));
    assert_eq!(
        Msg::decode(&t.take_outbox()[0]),
        Some(Msg::Ack {
            frame_id: 1,
            next: count
        })
    );
    assert!(t.take_events().is_empty());
}

#[test]
fn renegotiation_requeues_the_frame_in_flight() {
    let mut p = Pair::new(244, 244);
    p.run(2, &mut clean(), &mut no_dup(), &|p| {
        p.term.is_open() && p.phone.is_open()
    });
    let f = frame(6000, 1, 5);
    p.term.send_frame(p.now, f.clone()).unwrap();
    // the phone sees the first window, then restarts with a smaller proposal
    let first_window = p.term.take_outbox();
    assert!(first_window.len() >= 2);
    p.phone = Link::phone(p.now, 100);
    p.run(120, &mut clean(), &mut no_dup(), &|p| {
        !Pair::frames(&p.p_events).is_empty()
    });
    assert_eq!(Pair::frames(&p.p_events), vec![f]);
    assert_eq!(
        p.t_events
            .iter()
            .filter(|e| matches!(e, Event::Opened { .. }))
            .count(),
        2
    );
    assert!(matches!(
        p.t_events.last(),
        Some(Event::Opened { chunk: 100 }) | Some(Event::Sent)
    ));
}

#[test]
fn bye_ends_the_session_both_ways_and_user_close_says_why() {
    let mut p = Pair::new(244, 244);
    p.run(2, &mut clean(), &mut no_dup(), &|p| {
        p.term.is_open() && p.phone.is_open()
    });
    p.term.bye(p.now, B_USER);
    p.run(2, &mut clean(), &mut no_dup(), &|p| p.phone.is_closed());
    assert!(p.term.is_closed() && p.phone.is_closed());
    assert!(p.p_events.contains(&Event::Closed(B_USER)));
    assert!(p.t_events.contains(&Event::Closed(B_USER)));
}
