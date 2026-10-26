//! The phone's Bluetooth LE session (contract C20): the central side of the frame channel. The link layer is
//! `sshub_sync_proto::media::ble_link::Link::phone` (sans-IO, shared with the terminal's tests); this object is the seam
//! to the platform. Kotlin owns the GATT connection: it writes every message of `take_outbox` to the RX characteristic in
//! order, feeds every notification of the TX characteristic to `on_message`, and calls `tick` about once a second.
//!
//! Rust has no clock here: every call takes `now_ms`, monotonic milliseconds from any origin, and the session turns it
//! into an `Instant` on a base fixed at construction. A `now_ms` that goes backwards is held at the last value.
//!
//! The page (`assets/companion/src/ble.mjs`) rules that `Link::phone` does not carry are kept here: a HELLO_ACK that
//! answers with a message size above the proposal ends the session (BYE error), and a frame that would need more than
//! 65535 chunks is refused before it is queued.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use sshub_sync_proto::media::ble_link::{
    chunk_count, Event, Link, Msg, A_BUSY, A_CRC, A_ORDER, A_TIMEOUT, A_TOO_LARGE, A_VERSION, BLE_CHUNK_MAX, BLE_CHUNK_MIN, BLE_MAX_FRAME_BYTES,
    B_BUSY, B_DONE, B_ERROR, B_TIMEOUT, B_USER,
};

use crate::error::{CoreError, Result};

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum BleState {
    /// Not started, or HELLO sent and the terminal has not answered.
    Hello,
    /// Handshake done: frames can go both ways.
    Open,
    /// Over; `reason` is a sentence for the screen.
    Closed { reason: String },
}

struct Inner {
    base: Instant,
    last_ms: u64,
    proposal: usize,
    link: Option<Link>,
    chunk: usize,
    frames: Vec<Vec<u8>>,
    closed: Option<String>,
    send_failure: Option<String>,
}

impl Inner {
    fn at(&mut self, now_ms: u64) -> Instant {
        self.last_ms = self.last_ms.max(now_ms);
        self.base + Duration::from_millis(self.last_ms)
    }

    /// Moves what the link reported into the fields Kotlin reads.
    fn drain(&mut self) {
        let Some(link) = self.link.as_mut() else { return };
        for ev in link.take_events() {
            match ev {
                Event::Opened { chunk } => self.chunk = chunk,
                Event::Frame(f) => self.frames.push(f),
                Event::Sent => {}
                Event::SendFailed(r) => self.send_failure = Some(abort_text(r).to_string()),
                Event::Closed(r) => self.closed = Some(bye_text(r).to_string()),
            }
        }
    }
}

fn abort_text(reason: u8) -> &'static str {
    match reason {
        A_CRC => "The terminal found the frame damaged.",
        A_TOO_LARGE => "The frame is larger than the terminal accepts.",
        A_BUSY => "The terminal is busy with another frame.",
        A_ORDER => "The terminal received the chunks out of order.",
        A_VERSION => "The terminal speaks another version of the Bluetooth channel.",
        A_TIMEOUT => "The terminal stopped acknowledging.",
        _ => "The terminal aborted the frame.",
    }
}

fn bye_text(reason: u8) -> &'static str {
    match reason {
        B_DONE => "The Bluetooth session is finished.",
        B_TIMEOUT => "The Bluetooth session timed out.",
        B_USER => "The Bluetooth session was closed by the user.",
        B_ERROR => "The Bluetooth session failed.",
        B_BUSY => "The terminal is busy with another phone.",
        _ => "The Bluetooth session ended.",
    }
}

#[derive(uniffi::Object)]
pub struct BleSession {
    inner: Mutex<Inner>,
}

impl BleSession {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[uniffi::export]
impl BleSession {
    /// `mtu` is the negotiated ATT MTU: the HELLO proposes `min(mtu - 3, 512)` bytes per message, at least 20.
    #[uniffi::constructor]
    pub fn new(mtu: u32) -> Arc<Self> {
        let proposal = (mtu as usize).saturating_sub(3).clamp(BLE_CHUNK_MIN, BLE_CHUNK_MAX);
        Arc::new(Self {
            inner: Mutex::new(Inner {
                base: Instant::now(),
                last_ms: 0,
                proposal,
                link: None,
                chunk: BLE_CHUNK_MIN,
                frames: Vec::new(),
                closed: None,
                send_failure: None,
            }),
        })
    }

    /// Sends HELLO (the first message of `take_outbox`). A second call does nothing.
    pub fn start(&self, now_ms: u64) {
        let mut s = self.lock();
        if s.link.is_none() {
            let now = s.at(now_ms);
            s.link = Some(Link::phone(now, s.proposal));
        }
    }

    /// One message from the TX characteristic.
    pub fn on_message(&self, bytes: Vec<u8>, now_ms: u64) {
        let mut s = self.lock();
        let now = s.at(now_ms);
        let proposal = s.proposal;
        let Some(link) = s.link.as_mut() else { return };
        // The terminal may lower the message size, never raise it above what was proposed (the link would accept up to 512
        // and every write above the MTU would fail).
        if let Some(Msg::HelloAck { chunk, .. }) = Msg::decode(&bytes) {
            if usize::from(chunk) > proposal && !link.is_closed() {
                link.bye(now, B_ERROR);
                s.drain();
                return;
            }
        }
        link.on_message(now, &bytes);
        s.drain();
    }

    /// Messages to write to the RX characteristic, in order.
    pub fn take_outbox(&self) -> Vec<Vec<u8>> {
        self.lock().link.as_mut().map(Link::take_outbox).unwrap_or_default()
    }

    /// Timers: handshake, acknowledgement retries, frame deadline, idle, keep-alive.
    pub fn tick(&self, now_ms: u64) {
        let mut s = self.lock();
        let now = s.at(now_ms);
        if let Some(link) = s.link.as_mut() {
            link.tick(now);
        }
        s.drain();
    }

    /// Queues a whole frame (kinds 1 to 10) for the terminal; one is in flight at a time and a newer one replaces a
    /// queued one that has not started. Whether it arrived is not reported on success; a refusal by the terminal shows in
    /// `take_send_failure`.
    pub fn send_frame(&self, frame: Vec<u8>, now_ms: u64) -> Result<()> {
        let mut s = self.lock();
        let now = s.at(now_ms);
        let chunk = s.chunk;
        let open = s.link.as_ref().is_some_and(Link::is_open);
        if !open {
            return Err(CoreError::bad("the Bluetooth session is not open"));
        }
        if frame.len() > BLE_MAX_FRAME_BYTES {
            return Err(CoreError::bad("the frame is larger than the terminal accepts"));
        }
        let count = chunk_count(frame.len(), chunk);
        if count > usize::from(u16::MAX) {
            return Err(CoreError::bad(format!(
                "the frame needs {count} chunks of {chunk} bytes, more than the 65535 a frame may have"
            )));
        }
        let Some(link) = s.link.as_mut() else { return Err(CoreError::bad("the Bluetooth session is not open")) };
        link.send_frame(now, frame).map_err(CoreError::bad)?;
        s.drain();
        Ok(())
    }

    /// Whole frames received from the terminal since the last call, each checked (`SWR1`, kind 1 to 10, CRC).
    pub fn take_frames(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.lock().frames)
    }

    /// Why the last frame sent was refused or never acknowledged, once.
    pub fn take_send_failure(&self) -> Option<String> {
        self.lock().send_failure.take()
    }

    pub fn state(&self) -> BleState {
        let s = self.lock();
        if let Some(reason) = &s.closed {
            return BleState::Closed { reason: reason.clone() };
        }
        match &s.link {
            Some(l) if l.is_open() => BleState::Open,
            Some(l) if l.is_closed() => BleState::Closed { reason: bye_text(B_ERROR).to_string() },
            _ => BleState::Hello,
        }
    }

    /// The page after its last step: BYE and done (the BYE is in `take_outbox`).
    pub fn bye(&self, now_ms: u64) {
        let mut s = self.lock();
        let now = s.at(now_ms);
        if let Some(link) = s.link.as_mut() {
            link.bye(now, B_DONE);
        }
        s.drain();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sshub_sync_proto::media::ble_link::Msg;

    fn frame(len: usize, kind: u8, seed: u8) -> Vec<u8> {
        let mut f = b"SWR1".to_vec();
        f.extend_from_slice(&[0, 1, kind]);
        f.extend((0..len.saturating_sub(7)).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)));
        f
    }

    /// A terminal link and the session, with a virtual clock for the terminal (its own base).
    struct Rig {
        s: Arc<BleSession>,
        t: Link,
        t0: Instant,
        t_frames: Vec<Vec<u8>>,
    }

    impl Rig {
        fn new(mtu: u32) -> Rig {
            let t0 = Instant::now();
            Rig { s: BleSession::new(mtu), t: Link::terminal(t0, 512), t0, t_frames: Vec::new() }
        }
        fn at(&self, ms: u64) -> Instant {
            self.t0 + Duration::from_millis(ms)
        }
        /// Delivers everything in flight until quiet; `drop_phone` decides which phone messages are lost.
        fn run(&mut self, ms: u64, drop_phone: &mut dyn FnMut(&[u8]) -> bool) {
            loop {
                let up = self.s.take_outbox();
                let down = self.t.take_outbox();
                if up.is_empty() && down.is_empty() {
                    break;
                }
                for m in up {
                    if !drop_phone(&m) {
                        self.t.on_message(self.at(ms), &m);
                    }
                }
                for m in down {
                    self.s.on_message(m, ms);
                }
                for e in self.t.take_events() {
                    if let Event::Frame(f) = e {
                        self.t_frames.push(f);
                    }
                }
            }
        }
        fn open(mtu: u32) -> Rig {
            let mut r = Rig::new(mtu);
            r.s.start(0);
            r.run(0, &mut |_| false);
            assert_eq!(r.s.state(), BleState::Open);
            r
        }
    }

    /// A session opened by a hand-written HELLO_ACK (no terminal link), for the rules about single messages.
    fn manual(chunk: u16) -> Arc<BleSession> {
        let s = BleSession::new(247);
        s.start(0);
        assert_eq!(s.take_outbox(), vec![Msg::Hello { version: 1, chunk: 244 }.encode()]);
        s.on_message(Msg::HelloAck { version: 1, chunk, window: 8 }.encode(), 10);
        assert_eq!(s.state(), BleState::Open);
        s
    }

    fn data_ids(msgs: &[Vec<u8>]) -> Vec<(u16, u16)> {
        msgs.iter().filter(|m| m[0] == 0x10).map(|m| (u16::from_be_bytes([m[1], m[2]]), u16::from_be_bytes([m[3], m[4]]))).collect()
    }

    #[test]
    fn hello_then_a_frame_down_and_a_frame_up() {
        let mut r = Rig::open(247);
        let down = frame(3000, 5, 1);
        r.t.send_frame(r.at(5), down.clone()).unwrap();
        r.run(5, &mut |_| false);
        assert_eq!(r.s.take_frames(), vec![down]);
        assert!(r.s.take_frames().is_empty());
        let up = frame(5000, 6, 2);
        r.s.send_frame(up.clone(), 10).unwrap();
        r.run(10, &mut |_| false);
        assert_eq!(r.t_frames, vec![up]);
        assert!(r.s.take_send_failure().is_none());
        // a large frame and a small one back to back (the second waits for the first)
        r.s.send_frame(frame(20_000, 7, 3), 20).unwrap();
        r.s.send_frame(frame(100, 8, 4), 20).unwrap();
        r.run(20, &mut |_| false);
        assert_eq!(r.t_frames.len(), 3);
    }

    #[test]
    fn the_proposal_follows_the_mtu() {
        let s = BleSession::new(23);
        s.start(0);
        assert_eq!(s.take_outbox(), vec![Msg::Hello { version: 1, chunk: 20 }.encode()]);
        let s = BleSession::new(517);
        s.start(0);
        assert_eq!(s.take_outbox(), vec![Msg::Hello { version: 1, chunk: 512 }.encode()]);
        let s = BleSession::new(0);
        s.start(0);
        assert_eq!(s.take_outbox(), vec![Msg::Hello { version: 1, chunk: 20 }.encode()]);
        // start twice: one HELLO
        s.start(5);
        assert!(s.take_outbox().is_empty());
    }

    #[test]
    fn nothing_works_before_the_handshake() {
        let s = BleSession::new(247);
        assert_eq!(s.state(), BleState::Hello);
        assert!(s.send_frame(frame(100, 6, 0), 0).is_err());
        assert!(s.take_outbox().is_empty());
        s.on_message(vec![0x22], 1);
        s.tick(2);
        s.start(0);
        assert_eq!(s.state(), BleState::Hello);
        assert!(s.send_frame(frame(100, 6, 0), 0).is_err(), "HELLO sent, no HELLO_ACK yet");
    }

    #[test]
    fn a_lost_chunk_is_resent_after_the_ack_timer() {
        let mut r = Rig::open(247);
        let up = frame(2000, 6, 9);
        r.s.send_frame(up.clone(), 100).unwrap();
        let mut first = true;
        r.run(100, &mut |m| {
            let lose = first && m[0] == 0x10;
            first = false;
            lose
        });
        assert!(r.t_frames.is_empty(), "the first chunk never arrived");
        r.s.tick(100 + 5_001);
        r.run(5_101, &mut |_| false);
        assert_eq!(r.t_frames, vec![up]);
    }

    #[test]
    fn a_frame_too_big_for_65535_chunks_is_refused() {
        // at a 20 byte message size a frame of 900000 bytes needs more than 65535 chunks
        let mut r = Rig::open(23);
        let big = frame(900_000, 5, 0);
        assert!(chunk_count(big.len(), 20) > 65_535);
        let e = r.s.send_frame(big, 5).unwrap_err();
        assert!(e.reason().contains("65535"), "{e}");
        // the session is unharmed
        r.s.send_frame(frame(300, 6, 1), 6).unwrap();
        r.run(6, &mut |_| false);
        assert_eq!(r.t_frames.len(), 1);
        // a frame that is not a protocol frame, and one over the size limit
        assert!(r.s.send_frame(b"hello world".to_vec(), 7).is_err());
        assert!(r.s.send_frame(frame(BLE_MAX_FRAME_BYTES + 1, 6, 0), 7).is_err());
    }

    #[test]
    fn an_ack_beyond_the_highest_chunk_sent_is_ignored() {
        let s = manual(244);
        s.send_frame(frame(2000, 6, 1), 20).unwrap();
        let sent = s.take_outbox();
        assert_eq!(data_ids(&sent).len(), 8, "one window of an 9 chunk frame");
        let count = u16::from_be_bytes([sent[0][5], sent[0][6]]);
        assert_eq!(count, 9);
        // the terminal claims the whole frame, which never left this phone
        s.on_message(Msg::Ack { frame_id: 1, next: 9 }.encode(), 30);
        assert!(s.take_outbox().is_empty());
        assert!(s.take_send_failure().is_none());
        // the honest ACK of the window lets the last chunk go
        s.on_message(Msg::Ack { frame_id: 1, next: 8 }.encode(), 40);
        assert_eq!(data_ids(&s.take_outbox()), vec![(1, 8)]);
    }

    #[test]
    fn a_repeated_ack_goes_back_once() {
        let s = manual(244);
        s.send_frame(frame(2000, 6, 1), 20).unwrap();
        s.take_outbox();
        // the receiver saw a gap: ACK(0) while eight chunks are out
        s.on_message(Msg::Ack { frame_id: 1, next: 0 }.encode(), 30);
        let again = data_ids(&s.take_outbox());
        assert_eq!(again, (0..8).map(|i| (1, i)).collect::<Vec<_>>());
        // a second duplicate waits for progress
        s.on_message(Msg::Ack { frame_id: 1, next: 0 }.encode(), 31);
        assert!(s.take_outbox().is_empty());
        // an ACK for another frame id counts for nothing
        s.on_message(Msg::Ack { frame_id: 9, next: 3 }.encode(), 32);
        assert!(s.take_outbox().is_empty());
    }

    #[test]
    fn a_hello_ack_while_open_restarts_under_a_new_frame_id() {
        let s = manual(244);
        s.send_frame(frame(2000, 6, 1), 20).unwrap();
        s.take_outbox();
        s.on_message(Msg::HelloAck { version: 1, chunk: 100, window: 4 }.encode(), 30);
        assert_eq!(s.state(), BleState::Open);
        let out = s.take_outbox();
        let ids = data_ids(&out);
        assert_eq!(ids.len(), 4, "the new window");
        assert!(ids.iter().all(|(id, _)| *id == 2), "queued again under a new frame id");
        assert_eq!(ids[0].1, 0);
        for m in out.iter().filter(|m| m[0] == 0x10) {
            assert!(m.len() <= 100);
        }
    }

    #[test]
    fn a_hello_ack_above_the_proposal_ends_the_session() {
        let s = BleSession::new(100); // proposal 97
        s.start(0);
        s.take_outbox();
        s.on_message(Msg::HelloAck { version: 1, chunk: 98, window: 8 }.encode(), 5);
        assert!(matches!(s.state(), BleState::Closed { .. }));
        let out = s.take_outbox();
        assert_eq!(out.last().unwrap(), &Msg::Bye { reason: B_ERROR }.encode());
        // unusable numbers and a wrong version also end it (link rules)
        let s = BleSession::new(247);
        s.start(0);
        s.take_outbox();
        s.on_message(Msg::HelloAck { version: 1, chunk: 10, window: 8 }.encode(), 5);
        assert!(matches!(s.state(), BleState::Closed { .. }));
        let s = BleSession::new(247);
        s.start(0);
        s.on_message(Msg::HelloAck { version: 2, chunk: 100, window: 8 }.encode(), 5);
        assert!(matches!(s.state(), BleState::Closed { .. }));
        // a smaller size is fine
        let s = BleSession::new(247);
        s.start(0);
        s.on_message(Msg::HelloAck { version: 1, chunk: 120, window: 8 }.encode(), 5);
        assert_eq!(s.state(), BleState::Open);
    }

    #[test]
    fn the_frame_timer_is_not_re_armed_by_a_repeated_first_chunk() {
        // the terminal sends a frame of several chunks; the first chunk is repeated late. The frame window of 120 s runs
        // from the first arrival, so the abort comes at 121 s, not 220 s.
        let mut r = Rig::open(247);
        r.t.send_frame(r.at(0), frame(4000, 5, 3)).unwrap();
        let chunks = r.t.take_outbox();
        let data: Vec<_> = chunks.into_iter().filter(|m| m[0] == 0x10).collect();
        assert!(data.len() >= 4);
        r.s.on_message(data[0].clone(), 1_000);
        r.s.take_outbox();
        for at in [40_000, 80_000, 100_000] {
            r.s.on_message(data[0].clone(), at); // a repeat of chunk 0 (and the link stays alive: it keeps hearing)
            r.s.take_outbox();
        }
        r.s.tick(120_500);
        assert!(!r.s.take_outbox().iter().any(|m| m[0] == 0x12), "not yet: 119.5 s after the first chunk");
        r.s.tick(121_100);
        let out = r.s.take_outbox();
        let abort = out.iter().find(|m| m[0] == 0x12).expect("an ABORT");
        assert_eq!(abort[3], A_TIMEOUT);
    }

    #[test]
    fn bye_and_the_terminal_hanging_up_close_the_session() {
        let mut r = Rig::open(247);
        r.s.bye(50);
        assert!(matches!(r.s.state(), BleState::Closed { .. }));
        r.run(50, &mut |_| false);
        assert!(r.t.is_closed(), "the terminal saw the BYE");
        assert!(r.s.send_frame(frame(100, 6, 0), 60).is_err());
        // the terminal ends it
        let mut r = Rig::open(247);
        r.t.bye(r.at(5), B_BUSY);
        r.run(5, &mut |_| false);
        assert_eq!(r.s.state(), BleState::Closed { reason: bye_text(B_BUSY).to_string() });
    }

    #[test]
    fn silence_ends_the_session_and_a_late_clock_value_is_held() {
        let s = BleSession::new(247);
        s.start(1_000);
        s.tick(500); // backwards: held at 1000, no panic
        assert_eq!(s.state(), BleState::Hello);
        s.tick(1_000 + 10_001); // HELLO unanswered for over 10 s
        assert_eq!(s.state(), BleState::Closed { reason: bye_text(B_TIMEOUT).to_string() });
        let out = s.take_outbox();
        assert_eq!(out.last().unwrap(), &Msg::Bye { reason: B_TIMEOUT }.encode());
        // an open session with no traffic for 90 s
        let s = manual(244);
        s.tick(10 + 91_000);
        assert!(matches!(s.state(), BleState::Closed { .. }));
    }

    #[test]
    fn a_refused_frame_is_reported_once() {
        let s = manual(244);
        s.send_frame(frame(300, 6, 1), 20).unwrap();
        s.take_outbox();
        s.on_message(Msg::Abort { frame_id: 1, reason: A_TOO_LARGE }.encode(), 30);
        assert_eq!(s.take_send_failure().as_deref(), Some(abort_text(A_TOO_LARGE)));
        assert!(s.take_send_failure().is_none());
    }
}
