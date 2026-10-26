//! Link layer of the Bluetooth frame channel (contract C20): messages, chunking, flow control, timers
//! and the session state machine. Pure and sans-IO (no radio, no threads, no clock of its own: every
//! call takes `now`), so it builds without the `bluetooth` feature and is tested in CI. `ble.rs`
//! drives it over a transport (BlueZ, CoreBluetooth, or a fake in the tests).
//!
//! The terminal is the GATT peripheral; the phone page (Web Bluetooth) is the central. The link
//! carries whole frames of kinds 1 to 10 and adds no security: every frame is signed or sealed end to
//! end already, so nothing here is trusted beyond "a frame is a frame": it must start with `"SWR1"`,
//! carry a known kind and fit `BLE_MAX_FRAME_BYTES`, and a length is checked before a byte is buffered.

use std::time::{Duration, Instant};

use rand_core::{OsRng, RngCore};

pub const SERVICE_UUID: &str = "b62f7d3f-0001-42dd-8371-6ffd96411583";
pub const RX_UUID: &str = "b62f7d3f-0002-42dd-8371-6ffd96411583";
pub const TX_UUID: &str = "b62f7d3f-0003-42dd-8371-6ffd96411583";
pub const INFO_UUID: &str = "b62f7d3f-0004-42dd-8371-6ffd96411583";

pub const PROTO: u8 = 1;
pub const BLE_ADVERTISE_SECS: u64 = 300;
pub const BLE_HELLO_SECS: u64 = 10;
pub const BLE_ACK_SECS: u64 = 5;
pub const BLE_RETRIES: u8 = 3;
pub const BLE_FRAME_SECS: u64 = 120;
pub const BLE_PING_SECS: u64 = 30;
pub const BLE_IDLE_SECS: u64 = 90;
pub const BLE_MAX_FRAME_BYTES: usize = crate::limits::MAX_FRAME_BYTES;
pub const BLE_CHUNK_DEFAULT: usize = 244;
pub const BLE_CHUNK_MIN: usize = 20;
pub const BLE_CHUNK_MAX: usize = 512;
pub const BLE_WINDOW: usize = 8;

const T_HELLO: u8 = 0x01;
const T_HELLO_ACK: u8 = 0x02;
const T_DATA: u8 = 0x10;
const T_ACK: u8 = 0x11;
const T_ABORT: u8 = 0x12;
const T_BYE: u8 = 0x20;
const T_PING: u8 = 0x22;

/// ABORT reasons.
pub const A_CRC: u8 = 1;
pub const A_TOO_LARGE: u8 = 2;
pub const A_BUSY: u8 = 3;
pub const A_ORDER: u8 = 4;
pub const A_VERSION: u8 = 5;
pub const A_TIMEOUT: u8 = 6;
/// BYE reasons.
pub const B_DONE: u8 = 0;
pub const B_TIMEOUT: u8 = 1;
pub const B_USER: u8 = 2;
pub const B_ERROR: u8 = 3;
pub const B_BUSY: u8 = 4;

/// First chunk header: type, frame id, idx, count, then len and crc.
const HEAD0: usize = 1 + 2 + 2 + 2 + 4 + 4;
const HEADN: usize = 1 + 2 + 2 + 2;

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// IEEE 802.3 CRC-32 (check value of "123456789" is 0xCBF43926).
pub fn crc32(data: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut t = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                k += 1;
            }
            t[i] = c;
            i += 1;
        }
        t
    };
    !data.iter().fold(!0u32, |c, b| {
        TABLE[((c ^ u32::from(*b)) & 0xff) as usize] ^ (c >> 8)
    })
}

/// A frame the channel may carry: `"SWR1" || version u16 || kind u8 || body`, kind 1 to 10.
pub fn valid_frame(f: &[u8]) -> bool {
    f.len() >= 7 && f[..4] == *b"SWR1" && (1..=10).contains(&f[6])
}

/// A fresh pointer for the person to pick the right entry in the phone's chooser: 4 Crockford base32
/// characters from the OS RNG, 20 bits, derived from nothing stable.
pub fn new_tag() -> String {
    let mut b = [0u8; 4];
    OsRng.fill_bytes(&mut b);
    b.iter()
        .map(|x| CROCKFORD[usize::from(x & 31)] as char)
        .collect()
}

/// The advertised local name. Nothing about the machine or the user goes in it.
pub fn local_name(tag: &str) -> String {
    format!("SSHub-{tag}")
}

/// The INFO characteristic: `proto u8 || max_frame u32 BE || tag(4 ASCII)`.
pub fn info_bytes(tag: &str) -> Vec<u8> {
    let mut v = vec![PROTO];
    v.extend_from_slice(&(BLE_MAX_FRAME_BYTES as u32).to_be_bytes());
    v.extend_from_slice(tag.as_bytes());
    v
}

/// Chunks of a frame of `len` bytes at message size `chunk`.
pub fn chunk_count(len: usize, chunk: usize) -> usize {
    1 + len.saturating_sub(chunk - HEAD0).div_ceil(chunk - HEADN)
}

/// One message on the wire (one GATT write or notification).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    Hello {
        version: u8,
        chunk: u16,
    },
    HelloAck {
        version: u8,
        chunk: u16,
        window: u8,
    },
    Data {
        frame_id: u16,
        idx: u16,
        count: u16,
        /// `(len, crc32)`, present exactly when `idx == 0`.
        head: Option<(u32, u32)>,
        payload: Vec<u8>,
    },
    Ack {
        frame_id: u16,
        next: u16,
    },
    Abort {
        frame_id: u16,
        reason: u8,
    },
    Bye {
        reason: u8,
    },
    Ping,
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Msg::Hello { version, chunk } => {
                let mut v = vec![T_HELLO, *version];
                v.extend_from_slice(&chunk.to_be_bytes());
                v
            }
            Msg::HelloAck {
                version,
                chunk,
                window,
            } => {
                let mut v = vec![T_HELLO_ACK, *version];
                v.extend_from_slice(&chunk.to_be_bytes());
                v.push(*window);
                v
            }
            Msg::Data {
                frame_id,
                idx,
                count,
                head,
                payload,
            } => {
                let mut v = vec![T_DATA];
                v.extend_from_slice(&frame_id.to_be_bytes());
                v.extend_from_slice(&idx.to_be_bytes());
                v.extend_from_slice(&count.to_be_bytes());
                if let Some((len, crc)) = head {
                    v.extend_from_slice(&len.to_be_bytes());
                    v.extend_from_slice(&crc.to_be_bytes());
                }
                v.extend_from_slice(payload);
                v
            }
            Msg::Ack { frame_id, next } => {
                let mut v = vec![T_ACK];
                v.extend_from_slice(&frame_id.to_be_bytes());
                v.extend_from_slice(&next.to_be_bytes());
                v
            }
            Msg::Abort { frame_id, reason } => {
                let mut v = vec![T_ABORT];
                v.extend_from_slice(&frame_id.to_be_bytes());
                v.push(*reason);
                v
            }
            Msg::Bye { reason } => vec![T_BYE, *reason],
            Msg::Ping => vec![T_PING],
        }
    }

    /// `None` for anything that is not exactly one well-formed message.
    pub fn decode(b: &[u8]) -> Option<Msg> {
        let (&t, rest) = b.split_first()?;
        Some(match (t, rest.len()) {
            (T_HELLO, 3) => Msg::Hello {
                version: rest[0],
                chunk: be16(&rest[1..]),
            },
            (T_HELLO_ACK, 4) => Msg::HelloAck {
                version: rest[0],
                chunk: be16(&rest[1..]),
                window: rest[3],
            },
            (T_DATA, n) if n >= 6 => {
                let (frame_id, idx, count) = (be16(rest), be16(&rest[2..]), be16(&rest[4..]));
                let (head, payload) = if idx == 0 {
                    if n < 14 {
                        return None;
                    }
                    (Some((be32(&rest[6..]), be32(&rest[10..]))), &rest[14..])
                } else {
                    (None, &rest[6..])
                };
                if payload.is_empty() {
                    return None;
                }
                Msg::Data {
                    frame_id,
                    idx,
                    count,
                    head,
                    payload: payload.to_vec(),
                }
            }
            (T_ACK, 4) => Msg::Ack {
                frame_id: be16(rest),
                next: be16(&rest[2..]),
            },
            (T_ABORT, 3) => Msg::Abort {
                frame_id: be16(rest),
                reason: rest[2],
            },
            (T_BYE, 1) => Msg::Bye { reason: rest[0] },
            (T_PING, 0) => Msg::Ping,
            _ => return None,
        })
    }
}

/// What a [`Link`] tells the transport driver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Handshake done; messages are now up to `chunk` bytes.
    Opened { chunk: usize },
    /// A complete, checked frame from the other side.
    Frame(Vec<u8>),
    /// The frame we were sending is acknowledged in full.
    Sent,
    /// The frame we were sending was refused or timed out (ABORT reason).
    SendFailed(u8),
    /// The session is over (BYE reason, ours or theirs).
    Closed(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    AwaitHello,
    AwaitAck,
    Open,
    Closed,
}

struct Tx {
    id: u16,
    data: Vec<u8>,
    crc: u32,
    count: usize,
    acked: usize,
    sent: usize,
    deadline: Instant,
    retries: u8,
    resent: bool,
    /// Highest chunk count ever sent: bounds what an ACK may claim.
    max_sent: usize,
    started: Instant,
    attempt: u8,
}

struct Rx {
    id: u16,
    len: usize,
    crc: u32,
    count: usize,
    buf: Vec<u8>,
    next: usize,
    since_ack: usize,
    started: Instant,
}

/// One session, either side. The terminal side is [`Link::terminal`]; the phone side exists for the
/// tests (the page implements it in JS).
pub struct Link {
    state: State,
    chunk: usize,
    window: usize,
    notify_cap: usize,
    tx: Option<Tx>,
    pending: Option<(Vec<u8>, u8)>,
    next_id: u16,
    rx: Option<Rx>,
    last_done: Option<(u16, usize)>,
    outbox: Vec<Vec<u8>>,
    events: Vec<Event>,
    now: Instant,
    born: Instant,
    last_heard: Instant,
    last_sent: Instant,
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

impl Link {
    /// A terminal-side session for a central that just connected. `notify_cap` is the largest message
    /// the transport can notify (ATT MTU minus 3).
    pub fn terminal(now: Instant, notify_cap: usize) -> Link {
        Link {
            state: State::AwaitHello,
            chunk: BLE_CHUNK_MIN,
            window: BLE_WINDOW,
            notify_cap,
            tx: None,
            pending: None,
            next_id: 1,
            rx: None,
            last_done: None,
            outbox: Vec::new(),
            events: Vec::new(),
            now,
            born: now,
            last_heard: now,
            last_sent: now,
        }
    }

    /// The phone side: sends HELLO with `proposal` as its message size.
    pub fn phone(now: Instant, proposal: usize) -> Link {
        let mut l = Link::terminal(now, BLE_CHUNK_MAX);
        l.state = State::AwaitAck;
        l.emit(Msg::Hello {
            version: PROTO,
            chunk: proposal as u16,
        });
        l
    }

    pub fn is_closed(&self) -> bool {
        self.state == State::Closed
    }

    pub fn is_open(&self) -> bool {
        self.state == State::Open
    }

    /// Messages to put on the wire, in order.
    pub fn take_outbox(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.outbox)
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    fn emit(&mut self, m: Msg) {
        self.last_sent = self.now;
        self.outbox.push(m.encode());
    }

    fn close(&mut self, reason: u8, tell: bool) {
        if self.state == State::Closed {
            return;
        }
        if tell {
            self.emit(Msg::Bye { reason });
        }
        self.state = State::Closed;
        self.tx = None;
        self.rx = None;
        self.pending = None;
        self.events.push(Event::Closed(reason));
    }

    /// End the session from our side (modal closed, user dropped the phone).
    pub fn bye(&mut self, now: Instant, reason: u8) {
        self.now = now;
        self.close(reason, true);
    }

    fn fail(&mut self, abort: u8) {
        self.emit(Msg::Abort {
            frame_id: 0,
            reason: abort,
        });
        self.close(B_ERROR, true);
    }

    /// Queue a frame (kinds 1 to 10) for the other side. One frame is in flight; a newer queued frame
    /// replaces an older one that has not started.
    pub fn send_frame(&mut self, now: Instant, frame: Vec<u8>) -> Result<(), &'static str> {
        self.now = now;
        if !valid_frame(&frame) {
            return Err("not a protocol frame");
        }
        if frame.len() > BLE_MAX_FRAME_BYTES {
            return Err("frame too large for the Bluetooth channel");
        }
        if self.state == State::Closed {
            return Err("the Bluetooth session is closed");
        }
        self.pending = Some((frame, 0));
        self.pump();
        Ok(())
    }

    fn negotiate(&mut self, proposal: usize) {
        self.chunk = proposal
            .min(BLE_CHUNK_DEFAULT)
            .min(self.notify_cap)
            .max(BLE_CHUNK_MIN);
        self.window = BLE_WINDOW;
        self.emit(Msg::HelloAck {
            version: PROTO,
            chunk: self.chunk as u16,
            window: self.window as u8,
        });
        self.state = State::Open;
        self.events.push(Event::Opened { chunk: self.chunk });
        self.pump();
    }

    /// Renegotiation: the frame in flight is dropped (ours is queued again under a new id), what
    /// was half received is forgotten.
    fn restart(&mut self) {
        if let Some(tx) = self.tx.take() {
            if self.pending.is_none() {
                self.pending = Some((tx.data, tx.attempt));
            }
        }
        self.rx = None;
        self.last_done = None;
    }

    /// Everything that arrived on the wire, one message at a time.
    pub fn on_message(&mut self, now: Instant, bytes: &[u8]) {
        self.now = now;
        if self.state == State::Closed {
            return;
        }
        self.last_heard = now;
        if bytes.len() > self.chunk {
            self.emit(Msg::Abort {
                frame_id: 0,
                reason: A_TOO_LARGE,
            });
            return;
        }
        let Some(msg) = Msg::decode(bytes) else {
            return;
        };
        match (self.state, msg) {
            (State::AwaitHello, Msg::Hello { version, chunk }) => {
                if version != PROTO {
                    self.fail(A_VERSION);
                } else {
                    self.negotiate(usize::from(chunk));
                }
            }
            (State::Open, Msg::Hello { version, chunk }) => {
                if version != PROTO {
                    self.fail(A_VERSION);
                    return;
                }
                // renegotiation: what was in flight is dropped, our side re-queues it
                self.restart();
                self.negotiate(usize::from(chunk));
            }
            // the phone side exists for tests; a second HELLO_ACK (the terminal renegotiated)
            // restarts it like the page does
            (
                State::AwaitAck | State::Open,
                Msg::HelloAck {
                    version,
                    chunk,
                    window,
                },
            ) => {
                if version != PROTO || usize::from(chunk) < BLE_CHUNK_MIN || window == 0 {
                    self.fail(A_VERSION);
                    return;
                }
                self.restart();
                self.chunk = usize::from(chunk).min(BLE_CHUNK_MAX);
                self.window = usize::from(window);
                self.state = State::Open;
                self.events.push(Event::Opened { chunk: self.chunk });
                self.pump();
            }
            (
                State::Open,
                Msg::Data {
                    frame_id,
                    idx,
                    count,
                    head,
                    payload,
                },
            ) => {
                self.on_data(frame_id, idx, count, head, payload);
            }
            (State::Open, Msg::Ack { frame_id, next }) => self.on_ack(frame_id, usize::from(next)),
            (State::Open, Msg::Abort { frame_id, reason }) => self.on_abort(frame_id, reason),
            (_, Msg::Bye { reason }) => self.close(reason, false),
            _ => {}
        }
    }

    fn abort_rx(&mut self, id: u16, reason: u8) {
        self.rx = None;
        self.emit(Msg::Abort {
            frame_id: id,
            reason,
        });
    }

    fn on_data(
        &mut self,
        id: u16,
        idx: u16,
        count: u16,
        head: Option<(u32, u32)>,
        payload: Vec<u8>,
    ) {
        let (first, rest) = (self.chunk - HEAD0, self.chunk - HEADN);
        let finished = self.last_done.filter(|d| d.0 == id);
        match &self.rx {
            Some(rx) if rx.id != id => {
                match finished {
                    Some((did, dc)) => self.emit(Msg::Ack {
                        frame_id: did,
                        next: dc as u16,
                    }),
                    None => self.emit(Msg::Abort {
                        frame_id: id,
                        reason: A_BUSY,
                    }),
                }
                return;
            }
            Some(_) => {}
            None => {
                // a repeat of a frame we already took (our ACK was lost): say so again
                if let Some((did, dc)) = finished {
                    self.emit(Msg::Ack {
                        frame_id: did,
                        next: dc as u16,
                    });
                    return;
                }
                let Some((len, crc)) = head else {
                    return;
                };
                let len = len as usize;
                // the length is judged before a byte is kept
                if !(7..=BLE_MAX_FRAME_BYTES).contains(&len)
                    || usize::from(count) != chunk_count(len, self.chunk)
                    || payload.len() != len.min(first)
                {
                    self.emit(Msg::Abort {
                        frame_id: id,
                        reason: A_TOO_LARGE,
                    });
                    return;
                }
                self.rx = Some(Rx {
                    id,
                    len,
                    crc,
                    count: usize::from(count),
                    buf: Vec::with_capacity(len),
                    next: 0,
                    since_ack: 0,
                    started: self.now,
                });
            }
        }
        let Some(rx) = self.rx.as_mut() else {
            return;
        };
        if usize::from(count) != rx.count {
            self.abort_rx(id, A_ORDER);
            return;
        }
        let idx = usize::from(idx);
        if idx != rx.next {
            // a duplicate or a gap: tell the sender where we are
            let next = rx.next as u16;
            self.emit(Msg::Ack { frame_id: id, next });
            return;
        }
        let want = if idx == 0 {
            rx.len.min(first)
        } else if idx + 1 < rx.count {
            rest
        } else {
            rx.len - rx.buf.len()
        };
        if payload.len() != want {
            self.abort_rx(id, A_TOO_LARGE);
            return;
        }
        rx.buf.extend_from_slice(&payload);
        rx.next += 1;
        rx.since_ack += 1;
        if rx.next == rx.count {
            let Some(rx) = self.rx.take() else {
                return;
            };
            if rx.buf.len() != rx.len || crc32(&rx.buf) != rx.crc {
                self.abort_rx(id, A_CRC);
            } else if !valid_frame(&rx.buf) {
                self.abort_rx(id, A_BUSY);
            } else {
                self.last_done = Some((id, rx.count));
                self.emit(Msg::Ack {
                    frame_id: id,
                    next: rx.count as u16,
                });
                self.events.push(Event::Frame(rx.buf));
            }
        } else if rx.since_ack >= self.window / 2 {
            rx.since_ack = 0;
            let next = rx.next as u16;
            self.emit(Msg::Ack { frame_id: id, next });
        }
    }

    fn on_ack(&mut self, id: u16, next: usize) {
        let now = self.now;
        let Some(tx) = self.tx.as_mut() else {
            return;
        };
        // an ACK may refer to anything sent at any time, also before a go-back
        if id != tx.id || next > tx.max_sent {
            return;
        }
        if next > tx.acked {
            tx.acked = next;
            tx.sent = tx.sent.max(next);
            tx.retries = 0;
            tx.resent = false;
            tx.deadline = now + secs(BLE_ACK_SECS);
            if next >= tx.count {
                self.tx = None;
                self.events.push(Event::Sent);
            }
        } else if next == tx.acked && tx.sent > tx.acked && !tx.resent {
            // the receiver saw a gap (or a repeat): go back once, then wait for progress
            tx.sent = tx.acked;
            tx.resent = true;
        }
        self.pump();
    }

    fn on_abort(&mut self, id: u16, reason: u8) {
        if self.rx.as_ref().is_some_and(|r| r.id == id) {
            self.rx = None;
        }
        if self.tx.as_ref().is_some_and(|t| t.id == id) {
            let Some(tx) = self.tx.take() else {
                return;
            };
            if reason == A_CRC && tx.attempt + 1 < BLE_RETRIES {
                if self.pending.is_none() {
                    self.pending = Some((tx.data, tx.attempt + 1));
                }
            } else {
                self.events.push(Event::SendFailed(reason));
            }
            self.pump();
        }
    }

    fn data_msg(&self, tx: &Tx, idx: usize) -> Msg {
        let (first, rest) = (self.chunk - HEAD0, self.chunk - HEADN);
        let (from, to) = if idx == 0 {
            (0, tx.data.len().min(first))
        } else {
            let from = first + (idx - 1) * rest;
            (from, tx.data.len().min(from + rest))
        };
        Msg::Data {
            frame_id: tx.id,
            idx: idx as u16,
            count: tx.count as u16,
            head: (idx == 0).then_some((tx.data.len() as u32, tx.crc)),
            payload: tx.data[from..to].to_vec(),
        }
    }

    /// Start the queued frame if none is in flight, and send as much of the window as is open.
    fn pump(&mut self) {
        if self.state != State::Open {
            return;
        }
        if self.tx.is_none() {
            if let Some((data, attempt)) = self.pending.take() {
                let count = chunk_count(data.len(), self.chunk);
                if count > usize::from(u16::MAX) {
                    // only at tiny message sizes: the count field is a u16
                    self.events.push(Event::SendFailed(A_TOO_LARGE));
                    return;
                }
                let id = self.next_id;
                self.next_id = if id == u16::MAX { 1 } else { id + 1 };
                self.tx = Some(Tx {
                    id,
                    crc: crc32(&data),
                    count,
                    data,
                    acked: 0,
                    sent: 0,
                    deadline: self.now + secs(BLE_ACK_SECS),
                    retries: 0,
                    resent: false,
                    max_sent: 0,
                    started: self.now,
                    attempt,
                });
            }
        }
        let Some(mut tx) = self.tx.take() else {
            return;
        };
        while tx.sent < tx.count && tx.sent - tx.acked < self.window {
            if tx.sent == tx.acked {
                tx.deadline = self.now + secs(BLE_ACK_SECS);
            }
            let m = self.data_msg(&tx, tx.sent);
            self.emit(m);
            tx.sent += 1;
            tx.max_sent = tx.max_sent.max(tx.sent);
        }
        self.tx = Some(tx);
    }

    /// Timers: handshake, acknowledgements, frame deadlines, idle, keep-alive.
    pub fn tick(&mut self, now: Instant) {
        self.now = now;
        match self.state {
            State::Closed => return,
            State::AwaitHello => {
                if now.duration_since(self.born) > secs(BLE_HELLO_SECS) {
                    self.close(B_TIMEOUT, true);
                }
                return;
            }
            State::AwaitAck => {
                if now.duration_since(self.born) > secs(BLE_HELLO_SECS) {
                    self.close(B_TIMEOUT, true);
                }
                return;
            }
            State::Open => {}
        }
        if now.duration_since(self.last_heard) > secs(BLE_IDLE_SECS) {
            self.close(B_TIMEOUT, true);
            return;
        }
        if let Some(rx) = &self.rx {
            if now.duration_since(rx.started) > secs(BLE_FRAME_SECS) {
                let id = rx.id;
                self.abort_rx(id, A_TIMEOUT);
            }
        }
        if let Some(tx) = self.tx.as_mut() {
            let id = tx.id;
            if now.duration_since(tx.started) > secs(BLE_FRAME_SECS) {
                self.tx = None;
                self.events.push(Event::SendFailed(A_TIMEOUT));
                self.emit(Msg::Abort {
                    frame_id: id,
                    reason: A_TIMEOUT,
                });
            } else if tx.sent > tx.acked && now >= tx.deadline {
                tx.retries += 1;
                if tx.retries > BLE_RETRIES {
                    self.emit(Msg::Abort {
                        frame_id: id,
                        reason: A_TIMEOUT,
                    });
                    self.events.push(Event::SendFailed(A_TIMEOUT));
                    self.close(B_TIMEOUT, true);
                    return;
                }
                tx.sent = tx.acked;
                tx.resent = true;
                tx.deadline = now + secs(BLE_ACK_SECS);
                self.pump();
            }
        }
        if self.state == State::Open && now.duration_since(self.last_sent) >= secs(BLE_PING_SECS) {
            self.emit(Msg::Ping);
        }
        self.pump();
    }
}

#[cfg(test)]
mod tests;
