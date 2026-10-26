//! The channel code end to end through a fake transport: the driver thread runs as in production, the
//! test plays the phone page with a link of its own and exchanges whole frames in both directions.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::super::ble_link::{Event, Link, Msg, B_DONE, B_USER};
use super::super::{ListenOpts, MediaEvent, MediaKind};
use super::{serve, BleCmd, Emitter, Poll, Transport};
use crate::sync::wire::{EnrollFrame, ShareResponseWire};

enum FakeIn {
    Connect(usize),
    Msg(Vec<u8>),
    Disconnect,
    Fail(String),
}

struct Fake {
    inbox: Receiver<FakeIn>,
    out: Sender<Vec<u8>>,
    adv: Arc<Mutex<Vec<bool>>>,
    dropped: Arc<AtomicUsize>,
}

impl Transport for Fake {
    fn poll(&mut self, wait: Duration) -> Poll {
        match self.inbox.recv_timeout(wait) {
            Ok(FakeIn::Connect(cap)) => Poll::Connected { notify_cap: cap },
            Ok(FakeIn::Msg(m)) => Poll::Message(m),
            Ok(FakeIn::Disconnect) => Poll::Disconnected,
            Ok(FakeIn::Fail(e)) => Poll::Failed(e),
            Err(RecvTimeoutError::Timeout) => Poll::Idle,
            Err(RecvTimeoutError::Disconnected) => Poll::Failed("test transport gone".into()),
        }
    }
    fn send(&mut self, msg: &[u8]) -> bool {
        self.out.send(msg.to_vec()).is_ok()
    }
    fn advertise(&mut self, on: bool) {
        self.adv.lock().unwrap().push(on);
    }
    fn disconnect(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

/// A driver thread plus the phone's side of the wire.
struct Rig {
    to_driver: Sender<FakeIn>,
    from_driver: Receiver<Vec<u8>>,
    cmds: Sender<BleCmd>,
    events: Receiver<MediaEvent>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    adv: Arc<Mutex<Vec<bool>>>,
    dropped: Arc<AtomicUsize>,
    phone: Option<Link>,
    phone_events: Vec<Event>,
    media: Vec<MediaEvent>,
}

impl Rig {
    fn start(opts: ListenOpts, window: Duration) -> Rig {
        let (to_driver, inbox) = channel();
        let (out, from_driver) = channel();
        let (cmds, cmd_rx) = channel();
        let (etx, events) = channel();
        let adv = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let mut fake = Fake {
            inbox,
            out,
            adv: adv.clone(),
            dropped: dropped.clone(),
        };
        let stop2 = stop.clone();
        let thread = std::thread::spawn(move || {
            let em = Emitter::new(MediaKind::Ble, etx, opts);
            serve(&stop2, &em, "7QK2", &cmd_rx, &mut fake, window);
        });
        Rig {
            to_driver,
            from_driver,
            cmds,
            events,
            stop,
            thread: Some(thread),
            adv,
            dropped,
            phone: None,
            phone_events: Vec::new(),
            media: Vec::new(),
        }
    }

    /// The phone page connects (subscribes) and sends HELLO.
    fn connect(&mut self, proposal: usize, cap: usize) {
        self.to_driver.send(FakeIn::Connect(cap)).unwrap();
        self.phone = Some(Link::phone(Instant::now(), proposal));
    }

    /// Moves messages both ways until `until` holds (or 10 s pass).
    fn pump(&mut self, until: impl Fn(&Rig) -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let now = Instant::now();
            if let Some(p) = self.phone.as_mut() {
                while let Ok(m) = self.from_driver.try_recv() {
                    p.on_message(now, &m);
                }
                p.tick(now);
                for m in p.take_outbox() {
                    let _ = self.to_driver.send(FakeIn::Msg(m));
                }
                self.phone_events.extend(p.take_events());
            }
            while let Ok(e) = self.events.try_recv() {
                self.media.push(e);
            }
            if until(self) || now > end {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn phone_send(&mut self, frame: Vec<u8>) {
        self.phone
            .as_mut()
            .unwrap()
            .send_frame(Instant::now(), frame)
            .unwrap();
    }

    fn replies(&self) -> Vec<Vec<u8>> {
        self.media
            .iter()
            .filter_map(|e| match e {
                MediaEvent::Reply(MediaKind::Ble, b) => Some(b.clone()),
                _ => None,
            })
            .collect()
    }

    fn statuses(&self) -> Vec<String> {
        self.media
            .iter()
            .filter_map(|e| match e {
                MediaEvent::Status(MediaKind::Ble, s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    fn phone_frames(&self) -> Vec<Vec<u8>> {
        self.phone_events
            .iter()
            .filter_map(|e| match e {
                Event::Frame(f) => Some(f.clone()),
                _ => None,
            })
            .collect()
    }

    fn join(&mut self) {
        self.thread.take().unwrap().join().unwrap();
    }

    fn finished(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| t.is_finished())
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn challenge_frame() -> Vec<u8> {
    // kind 1, as a QR would carry it (the channel only checks magic and kind)
    let mut f = b"SWR1".to_vec();
    f.extend_from_slice(&[0, 1, 1]);
    f.extend((0..300u32).map(|i| (i * 7) as u8));
    f
}

fn reply_wire(request_id: u64, ct: usize) -> Vec<u8> {
    ShareResponseWire {
        request_id,
        ciphertext: (0..ct).map(|i| (i * 13 + 5) as u8).collect(),
    }
    .encode()
}

#[test]
fn unlock_exchange_challenge_out_reply_in_and_the_channel_ends() {
    let mut rig = Rig::start(ListenOpts::SHARE_RESPONSE, Duration::from_secs(60));
    // the modal queues the challenge before any phone shows up
    rig.cmds.send(BleCmd::Frame(challenge_frame())).unwrap();
    rig.connect(244, 244);
    rig.pump(|r| !r.phone_frames().is_empty());
    assert_eq!(rig.phone_frames(), vec![challenge_frame()]);
    let reply = reply_wire(4242, 700);
    rig.phone_send(reply.clone());
    rig.pump(|r| r.finished());
    assert_eq!(rig.replies(), vec![reply]);
    rig.join();
    rig.pump(|r| r.phone_events.iter().any(|e| matches!(e, Event::Closed(_))));
    assert!(
        rig.phone_events.contains(&Event::Closed(B_DONE)),
        "{:?}",
        rig.phone_events
    );
    assert!(rig.phone_events.contains(&Event::Sent));
    let adv = rig.adv.lock().unwrap().clone();
    assert_eq!(
        adv,
        vec![true, false],
        "advertise, then stop at the connection"
    );
    assert!(
        rig.dropped.load(Ordering::SeqCst) >= 1,
        "the central is dropped at the end"
    );
    let st = rig.statuses();
    assert_eq!(st[0], "Bluetooth: pick SSHub-7QK2 in the phone's list");
    assert!(
        st.iter()
            .any(|s| s.contains("phone connected (messages of 244 bytes)")),
        "{st:?}"
    );
}

#[test]
fn a_big_reply_with_courier_ops_crosses_in_chunks() {
    let mut rig = Rig::start(ListenOpts::SHARE_RESPONSE, Duration::from_secs(60));
    rig.connect(185, 244);
    rig.pump(|r| r.phone.as_ref().is_some_and(|p| p.is_open()));
    // 100 KB: far above anything real, still one frame
    let reply = reply_wire(1, 100_000);
    rig.phone_send(reply.clone());
    rig.pump(|r| r.finished());
    assert_eq!(rig.replies(), vec![reply]);
}

#[test]
fn enrollment_steps_in_a_row_across_a_reconnect() {
    let mut rig = Rig::start(ListenOpts::ENROLL_FRAMES, Duration::from_secs(60));
    let commit = EnrollFrame::Commit {
        role: 0,
        commit: [7; 32],
    }
    .encode();
    let reveal = EnrollFrame::Reveal {
        role: 0,
        reveal: (0..1300u32).map(|i| (i * 13 + 5) as u8).collect(),
    }
    .encode();
    let confirm = EnrollFrame::Confirm {
        role: 0,
        ok: true,
        sig: [9; 64],
    }
    .encode();
    let ours = EnrollFrame::Commit {
        role: 1,
        commit: [3; 32],
    }
    .encode();
    rig.cmds.send(BleCmd::Frame(ours.clone())).unwrap();
    rig.connect(244, 244);
    rig.pump(|r| !r.phone_frames().is_empty());
    assert_eq!(rig.phone_frames(), vec![ours]);
    rig.phone_send(commit.clone());
    rig.phone_send(reveal.clone());
    rig.pump(|r| !r.replies().is_empty());
    // one frame in flight at a time: the second goes after the first is acknowledged
    rig.pump(|r| r.replies().len() >= 2);
    assert_eq!(rig.replies(), vec![commit.clone(), reveal.clone()]);
    assert!(!rig.finished(), "enrollment keeps listening");
    // the phone drops off and comes back: the channel advertises again and takes the next step
    rig.to_driver.send(FakeIn::Disconnect).unwrap();
    rig.pump(|r| r.adv.lock().unwrap().len() >= 3);
    assert_eq!(rig.adv.lock().unwrap().clone(), vec![true, false, true]);
    let step2 = EnrollFrame::Reveal {
        role: 1,
        reveal: vec![1; 40],
    }
    .encode();
    rig.cmds.send(BleCmd::Frame(step2.clone())).unwrap();
    rig.connect(244, 244);
    rig.pump(|r| r.phone_frames().len() >= 2);
    assert_eq!(
        rig.phone_frames().last(),
        Some(&step2),
        "the current step is pushed again"
    );
    rig.phone_send(confirm.clone());
    rig.pump(|r| r.replies().len() >= 3);
    assert_eq!(rig.replies(), vec![commit, reveal, confirm]);
    // closing the modal says goodbye
    rig.stop.store(true, Ordering::SeqCst);
    rig.pump(|r| r.finished());
    rig.pump(|r| r.phone_events.iter().any(|e| matches!(e, Event::Closed(_))));
    assert!(rig.phone_events.contains(&Event::Closed(B_USER)));
}

#[test]
fn a_frame_the_step_does_not_expect_is_reported_and_ignored() {
    let mut rig = Rig::start(ListenOpts::SHARE_RESPONSE, Duration::from_secs(60));
    rig.connect(244, 244);
    rig.pump(|r| r.phone.as_ref().is_some_and(|p| p.is_open()));
    // a valid frame of another kind than the step wants (an enrollment commit while unlocking)
    rig.phone_send(
        EnrollFrame::Commit {
            role: 0,
            commit: [1; 32],
        }
        .encode(),
    );
    rig.pump(|r| r.statuses().iter().any(|s| s.contains("does not expect")));
    assert!(rig.replies().is_empty());
    assert!(!rig.finished());
    // the right frame then still works
    let reply = reply_wire(9, 100);
    rig.phone_send(reply.clone());
    rig.pump(|r| r.finished());
    assert_eq!(rig.replies(), vec![reply]);
}

#[test]
fn dropping_the_central_says_bye_and_advertises_again() {
    let mut rig = Rig::start(ListenOpts::SHARE_RESPONSE, Duration::from_secs(60));
    rig.connect(244, 244);
    rig.pump(|r| r.phone.as_ref().is_some_and(|p| p.is_open()));
    rig.cmds.send(BleCmd::DropCentral).unwrap();
    rig.pump(|r| {
        r.phone_events
            .iter()
            .any(|e| matches!(e, Event::Closed(B_USER)))
    });
    rig.pump(|r| r.adv.lock().unwrap().len() >= 3);
    assert_eq!(rig.adv.lock().unwrap().clone(), vec![true, false, true]);
    assert!(rig.dropped.load(Ordering::SeqCst) >= 1);
    assert!(!rig.finished());
}

#[test]
fn a_dead_radio_is_reported_and_ends_the_channel() {
    let mut rig = Rig::start(ListenOpts::SHARE_RESPONSE, Duration::from_secs(60));
    rig.to_driver
        .send(FakeIn::Fail("bluetoothd is not running".into()))
        .unwrap();
    rig.pump(|r| r.finished());
    assert!(rig
        .statuses()
        .iter()
        .any(|s| s == "Bluetooth unavailable: bluetoothd is not running"));
}

#[test]
fn the_advertising_window_ends_the_channel_when_nobody_connects() {
    let mut rig = Rig::start(ListenOpts::SHARE_RESPONSE, Duration::from_millis(300));
    rig.pump(|r| r.finished());
    assert!(rig
        .statuses()
        .iter()
        .any(|s| s.contains("advertising ended")));
    assert_eq!(rig.adv.lock().unwrap().clone(), vec![true, false]);
}

#[test]
fn a_central_that_never_says_hello_is_not_a_session_and_garbage_is_ignored() {
    let mut rig = Rig::start(ListenOpts::SHARE_RESPONSE, Duration::from_secs(60));
    rig.to_driver.send(FakeIn::Connect(244)).unwrap();
    rig.to_driver
        .send(FakeIn::Msg(vec![0x55, 1, 2, 3]))
        .unwrap();
    rig.to_driver
        .send(FakeIn::Msg(
            Msg::Ack {
                frame_id: 1,
                next: 1,
            }
            .encode(),
        ))
        .unwrap();
    rig.pump(|r| {
        r.statuses()
            .iter()
            .any(|s| s.contains("phone connected, handshaking"))
    });
    assert!(
        rig.from_driver.try_recv().is_err(),
        "nothing is answered before HELLO"
    );
    assert!(rig.replies().is_empty());
}

#[test]
fn the_newest_queued_frame_is_what_a_late_phone_receives() {
    let mut rig = Rig::start(ListenOpts::SHARE_RESPONSE, Duration::from_secs(60));
    let old = challenge_frame();
    let mut new = challenge_frame();
    new[7 + 5] ^= 0xff;
    rig.cmds.send(BleCmd::Frame(old)).unwrap();
    rig.cmds.send(BleCmd::Frame(new.clone())).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    rig.connect(244, 244);
    rig.pump(|r| !r.phone_frames().is_empty());
    assert_eq!(rig.phone_frames(), vec![new]);
}
