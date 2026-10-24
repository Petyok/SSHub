//! Length-prefixed JSON frames: a `u32` big-endian length, then a UTF-8 JSON
//! body. Ported from the measured PoC F rig.
//!
//! Every cap is enforced before the cost it guards: the length prefix is
//! checked against [`limits::MAX_FRAME_BYTES`] before the body is allocated,
//! and the nesting depth against [`limits::MAX_JSON_DEPTH`] by a byte scan that
//! runs before `serde_json` sees the body.

use std::fmt;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::limits::{self, MAX_FRAME_BYTES, MAX_JSON_DEPTH};

/// Hard deadline from the start of a connection to the peer's hello (PoC F:
/// `ConnectTimeout=4` plus 500 ms keeps an offline verdict under five seconds).
pub const CONNECT_TO_HELLO: Duration = Duration::from_millis(4500);
/// How often a blocked wait looks at the cancel flag.
pub(crate) const CANCEL_POLL: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub enum FrameError {
    Oversize(u64),
    Empty,
    Truncated,
    Depth(usize),
    Utf8,
    Json(String),
    Io(io::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Oversize(n) => {
                write!(
                    f,
                    "length prefix {n} exceeds the {MAX_FRAME_BYTES} byte frame cap"
                )
            }
            FrameError::Empty => write!(f, "zero-length frame"),
            FrameError::Truncated => write!(f, "stream ended inside a frame"),
            FrameError::Depth(d) => {
                write!(
                    f,
                    "JSON nesting depth {d} exceeds the cap of {MAX_JSON_DEPTH}"
                )
            }
            FrameError::Utf8 => write!(f, "frame body is not valid UTF-8"),
            FrameError::Json(e) => write!(f, "frame body is not valid JSON: {e}"),
            FrameError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Serialise `body` and write it as one frame, flushing.
pub fn write_frame<W: Write>(w: &mut W, body: &serde_json::Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(body).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame too large",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_be_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

/// Read one frame. `Ok(None)` is a clean end of stream at a frame boundary;
/// an end of stream anywhere else is [`FrameError::Truncated`]. Partial reads
/// are resumed, so a peer dribbling bytes is fine.
pub fn read_frame<R: Read>(r: &mut R) -> Result<Option<serde_json::Value>, FrameError> {
    let mut hdr = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut hdr[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(FrameError::Io(e)),
        }
    }
    let len = u32::from_be_bytes(hdr) as usize;
    // Before any allocation: a hostile prefix costs four bytes of reading.
    if len > MAX_FRAME_BYTES {
        return Err(FrameError::Oversize(len as u64));
    }
    if len == 0 {
        return Err(FrameError::Empty);
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => FrameError::Truncated,
        _ => FrameError::Io(e),
    })?;
    check_depth(&body)?;
    let text = std::str::from_utf8(&body).map_err(|_| FrameError::Utf8)?;
    serde_json::from_str(text)
        .map(Some)
        .map_err(|e| FrameError::Json(e.to_string()))
}

/// Byte scan that tracks string state, so brackets inside strings do not count.
/// Stops at the first byte past the cap, so a 10 000-deep body is reported at
/// depth `MAX_JSON_DEPTH + 1` without walking the rest.
pub fn check_depth(body: &[u8]) -> Result<(), FrameError> {
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for &c in body {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_JSON_DEPTH {
                    return Err(FrameError::Depth(depth));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

// ── Connection ───────────────────────────────────────────

/// Deadlines of one connection.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// From [`Conn::new`] to the peer's hello.
    pub hello: Duration,
    /// Longest silence while waiting for a frame.
    pub idle: Duration,
    /// The whole round, from [`Conn::new`].
    pub round: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            hello: CONNECT_TO_HELLO,
            idle: Duration::from_secs(limits::IDLE_TIMEOUT_SECS),
            round: Duration::from_secs(limits::ROUND_TIMEOUT_SECS),
        }
    }
}

#[derive(Debug)]
pub enum ConnError {
    /// A deadline passed.
    Timeout,
    /// The peer closed the stream.
    Closed,
    /// The worker is shutting down.
    Cancelled,
    Frame(FrameError),
    /// A well-formed frame that is not the message type asked for.
    Decode(String),
    Io(io::Error),
}

impl fmt::Display for ConnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnError::Timeout => write!(f, "timed out waiting for the peer"),
            ConnError::Closed => write!(f, "the peer closed the connection"),
            ConnError::Cancelled => write!(f, "cancelled"),
            ConnError::Frame(e) => write!(f, "bad frame from the peer: {e}"),
            ConnError::Decode(e) => write!(f, "unexpected message from the peer: {e}"),
            ConnError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for ConnError {}

type Incoming = Result<Option<serde_json::Value>, FrameError>;

/// A framed, deadline-bound connection over any reader and writer: ssh child
/// pipes on the client, stdin and stdout in `sync-serve`. A reader thread
/// turns the peer's byte stream into frames, so waits can time out and be
/// cancelled; writes go straight to the peer. Frames are JSON: byte fields in
/// a message type travel as base64.
pub struct Conn {
    writer: Box<dyn Write + Send>,
    rx: Receiver<Incoming>,
    started: Instant,
    limits: Limits,
    cancel: Arc<AtomicBool>,
    hello_seen: Arc<AtomicBool>,
}

impl Conn {
    pub fn new(
        mut reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
        limits: Limits,
        cancel: Arc<AtomicBool>,
    ) -> Conn {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || loop {
            let frame = read_frame(&mut reader);
            let last = !matches!(frame, Ok(Some(_)));
            if tx.send(frame).is_err() || last {
                break;
            }
        });
        Conn {
            writer: Box::new(writer),
            rx,
            started: Instant::now(),
            limits,
            cancel,
            hello_seen: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Send one message as a frame.
    pub fn send_msg<T: Serialize>(&mut self, msg: &T) -> Result<(), ConnError> {
        let value = serde_json::to_value(msg).map_err(|e| ConnError::Decode(e.to_string()))?;
        self.send(&value)
    }

    /// The peer's first message, within the connect-to-hello deadline.
    pub fn recv_hello_msg<T: DeserializeOwned>(&mut self) -> Result<T, ConnError> {
        decode(self.recv_hello()?)
    }

    /// The next message: within the idle timeout and the round deadline.
    pub fn recv_msg<T: DeserializeOwned>(&mut self) -> Result<T, ConnError> {
        decode(self.recv()?)
    }

    /// [`Conn::send_msg`] for a raw JSON value.
    pub fn send(&mut self, frame: &serde_json::Value) -> Result<(), ConnError> {
        write_frame(&mut self.writer, frame).map_err(ConnError::Io)
    }

    /// [`Conn::recv_hello_msg`] for a raw JSON value.
    pub fn recv_hello(&mut self) -> Result<serde_json::Value, ConnError> {
        let left = (self.started + self.limits.hello).saturating_duration_since(Instant::now());
        let hello = self.wait(left)?;
        self.hello_seen.store(true, Ordering::SeqCst);
        Ok(hello)
    }

    /// [`Conn::recv_msg`] for a raw JSON value.
    pub fn recv(&mut self) -> Result<serde_json::Value, ConnError> {
        let round_left =
            (self.started + self.limits.round).saturating_duration_since(Instant::now());
        self.wait(self.limits.idle.min(round_left))
    }

    fn wait(&mut self, within: Duration) -> Result<serde_json::Value, ConnError> {
        let deadline = Instant::now() + within;
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Err(ConnError::Cancelled);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ConnError::Timeout);
            }
            match self.rx.recv_timeout(left.min(CANCEL_POLL)) {
                Ok(Ok(Some(v))) => return Ok(v),
                Ok(Ok(None)) | Err(RecvTimeoutError::Disconnected) => {
                    return Err(ConnError::Closed)
                }
                Ok(Err(e)) => return Err(ConnError::Frame(e)),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// Whether the peer's hello arrived (what separates "offline" from "failed").
    pub fn hello_seen(&self) -> bool {
        self.hello_seen.load(Ordering::SeqCst)
    }

    /// The hello flag, for a watchdog that must not hold the connection.
    pub(crate) fn hello_flag(&self) -> Arc<AtomicBool> {
        self.hello_seen.clone()
    }
}

fn decode<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, ConnError> {
    serde_json::from_value(value).map_err(|e| ConnError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut v = (body.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(body);
        v
    }

    fn nested(d: usize) -> Vec<u8> {
        let mut b = vec![b'['; d];
        b.extend(std::iter::repeat_n(b']', d));
        frame(&b)
    }

    /// The PoC F malformed-frame list (F4): raw bytes, and whether the reader
    /// must accept them. The last entry is the control at the depth cap.
    fn malformed_cases() -> Vec<(&'static str, Vec<u8>, bool)> {
        vec![
            (
                "length prefix 1 MiB + 1 (no body)",
                ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes().to_vec(),
                false,
            ),
            (
                "length prefix 0xFFFFFFFF (4 GiB, no body)",
                vec![0xff, 0xff, 0xff, 0xff],
                false,
            ),
            (
                "length prefix 1 GiB, 2 bytes of body",
                {
                    let mut v = 0x4000_0000u32.to_be_bytes().to_vec();
                    v.extend_from_slice(b"{}");
                    v
                },
                false,
            ),
            ("nested arrays depth 10000", nested(10_000), false),
            ("nested arrays depth 33 (one over cap)", nested(33), false),
            (
                "truncated: prefix says 100, 10 bytes follow",
                {
                    let mut v = 100u32.to_be_bytes().to_vec();
                    v.extend_from_slice(b"{\"type\":\"");
                    v
                },
                false,
            ),
            (
                "invalid UTF-8 body",
                frame(&[0xff, 0xfe, b'{', b'}']),
                false,
            ),
            ("trailing garbage after JSON", frame(b"{}xyz"), false),
            ("zero-length frame", frame(&[]), false),
            (
                "control: nested arrays depth 32 (accepted)",
                nested(32),
                true,
            ),
        ]
    }

    #[test]
    fn the_poc_f_malformed_list_is_rejected_and_the_control_accepted() {
        for (name, bytes, ok) in malformed_cases() {
            let got = read_frame(&mut Cursor::new(bytes));
            assert_eq!(got.is_ok(), ok, "{name}: {got:?}");
        }
    }

    #[test]
    fn each_malformed_case_fails_for_its_own_reason() {
        let kind = |bytes: Vec<u8>| match read_frame(&mut Cursor::new(bytes)) {
            Err(FrameError::Oversize(_)) => "oversize",
            Err(FrameError::Empty) => "empty",
            Err(FrameError::Truncated) => "truncated",
            Err(FrameError::Depth(_)) => "depth",
            Err(FrameError::Utf8) => "utf8",
            Err(FrameError::Json(_)) => "json",
            other => panic!("unexpected {other:?}"),
        };
        let cases = malformed_cases();
        let want = [
            "oversize",
            "oversize",
            "oversize",
            "depth",
            "depth",
            "truncated",
            "utf8",
            "json",
            "empty",
        ];
        for ((name, bytes, _), want) in cases.into_iter().zip(want) {
            assert_eq!(kind(bytes), want, "{name}");
        }
    }

    #[test]
    fn an_oversize_prefix_is_refused_without_reading_or_allocating_the_body() {
        // A reader that panics if asked for more than the four prefix bytes.
        struct Header(Vec<u8>);
        impl Read for Header {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                assert!(self.0.len() <= 4 && buf.len() <= 4, "body was requested");
                let n = self.0.len().min(buf.len());
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0.drain(..n);
                Ok(n)
            }
        }
        let mut r = Header(0xFFFF_FFFFu32.to_be_bytes().to_vec());
        assert!(matches!(read_frame(&mut r), Err(FrameError::Oversize(n)) if n == 0xFFFF_FFFF));
    }

    #[test]
    fn the_depth_cap_is_exact_and_ignores_brackets_inside_strings() {
        assert!(check_depth(&[b'['; MAX_JSON_DEPTH]).is_ok());
        assert!(matches!(
            check_depth(&[b'['; MAX_JSON_DEPTH + 1]),
            Err(FrameError::Depth(d)) if d == MAX_JSON_DEPTH + 1
        ));
        let deep_string = format!("\"{}\\\"{}\"", "[".repeat(100), "{".repeat(100));
        assert!(check_depth(deep_string.as_bytes()).is_ok());
    }

    #[test]
    fn frames_round_trip_and_end_cleanly() {
        let a = serde_json::json!({"type": "hello", "n": [1, 2, {"k": "v"}]});
        let b = serde_json::json!({"type": "bye"});
        let mut wire = Vec::new();
        write_frame(&mut wire, &a).unwrap();
        write_frame(&mut wire, &b).unwrap();
        let mut r = Cursor::new(wire);
        assert_eq!(read_frame(&mut r).unwrap(), Some(a));
        assert_eq!(read_frame(&mut r).unwrap(), Some(b));
        assert!(
            read_frame(&mut r).unwrap().is_none(),
            "clean EOF at a boundary"
        );
    }

    #[test]
    fn partial_reads_are_resumed() {
        struct OneByte(Cursor<Vec<u8>>);
        impl Read for OneByte {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = buf.len().min(1);
                self.0.read(&mut buf[..n])
            }
        }
        let v = serde_json::json!({"type": "tail", "ops": [1, 2, 3]});
        let mut wire = Vec::new();
        write_frame(&mut wire, &v).unwrap();
        let mut r = OneByte(Cursor::new(wire));
        assert_eq!(read_frame(&mut r).unwrap(), Some(v));
    }

    #[test]
    fn eof_inside_the_prefix_or_the_body_is_truncation_not_a_clean_end() {
        assert!(matches!(
            read_frame(&mut Cursor::new(vec![0, 0])),
            Err(FrameError::Truncated)
        ));
        let mut wire = frame(b"{\"a\":1}");
        wire.truncate(wire.len() - 1);
        assert!(matches!(
            read_frame(&mut Cursor::new(wire)),
            Err(FrameError::Truncated)
        ));
    }

    #[test]
    fn a_stray_byte_after_a_valid_frame_is_an_error_on_the_next_read() {
        let mut wire = frame(b"{}");
        wire.push(0x07);
        let mut r = Cursor::new(wire);
        assert!(read_frame(&mut r).unwrap().is_some());
        assert!(matches!(read_frame(&mut r), Err(FrameError::Truncated)));
    }

    #[test]
    fn a_body_over_the_cap_is_not_written() {
        let big = serde_json::Value::String("x".repeat(MAX_FRAME_BYTES));
        let mut sink = Vec::new();
        let err = write_frame(&mut sink, &big).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(sink.is_empty(), "nothing reached the wire");
    }
}

#[cfg(test)]
mod conn_tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;
    use std::os::unix::net::UnixStream;

    fn quick() -> Limits {
        Limits {
            hello: Duration::from_millis(200),
            idle: Duration::from_millis(200),
            round: Duration::from_secs(5),
        }
    }

    fn pair(limits: Limits) -> (Conn, UnixStream, Arc<AtomicBool>) {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let conn = Conn::new(ours.try_clone().unwrap(), ours, limits, cancel.clone());
        (conn, theirs, cancel)
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Msg {
        Hello { version: u16 },
        Bye,
    }

    #[test]
    fn typed_messages_round_trip_over_an_in_process_pipe() {
        let (mut conn, mut peer, _) = pair(quick());
        let t = thread::spawn(move || {
            let m = read_frame(&mut peer).unwrap().unwrap();
            write_frame(&mut peer, &m).unwrap(); // echo
        });
        conn.send_msg(&Msg::Hello { version: 1 }).unwrap();
        assert_eq!(
            conn.recv_hello_msg::<Msg>().unwrap(),
            Msg::Hello { version: 1 }
        );
        assert!(conn.hello_seen());
        t.join().unwrap();
    }

    #[test]
    fn a_message_of_the_wrong_shape_is_a_decode_error_not_a_panic() {
        let (mut conn, mut peer, _) = pair(quick());
        write_frame(&mut peer, &json!({"type": "nonsense"})).unwrap();
        assert!(matches!(conn.recv_msg::<Msg>(), Err(ConnError::Decode(_))));
    }

    #[test]
    fn a_silent_peer_misses_the_hello_deadline_and_the_idle_timeout() {
        let (mut conn, _peer, _) = pair(quick());
        let start = Instant::now();
        assert!(matches!(
            conn.recv_hello_msg::<Msg>(),
            Err(ConnError::Timeout)
        ));
        assert!(!conn.hello_seen());
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(matches!(conn.recv_msg::<Msg>(), Err(ConnError::Timeout)));
    }

    #[test]
    fn the_round_deadline_cuts_a_chatty_connection_short() {
        let limits = Limits {
            round: Duration::from_millis(150),
            ..quick()
        };
        let (mut conn, _peer, _) = pair(limits);
        thread::sleep(Duration::from_millis(200));
        assert!(matches!(conn.recv_msg::<Msg>(), Err(ConnError::Timeout)));
    }

    #[test]
    fn eof_and_bad_frames_surface_as_errors() {
        let (mut conn, peer, _) = pair(quick());
        drop(peer);
        assert!(matches!(conn.recv_msg::<Msg>(), Err(ConnError::Closed)));
        assert!(
            matches!(conn.recv_msg::<Msg>(), Err(ConnError::Closed)),
            "and stays closed"
        );

        let (mut conn, mut peer, _) = pair(quick());
        peer.write_all(&[0xff, 0xff, 0xff, 0xff]).unwrap();
        assert!(matches!(
            conn.recv_msg::<Msg>(),
            Err(ConnError::Frame(FrameError::Oversize(_)))
        ));
    }

    #[test]
    fn cancelling_wakes_a_blocked_wait() {
        let (mut conn, _peer, cancel) = pair(Limits::default());
        let flag = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            flag.store(true, Ordering::SeqCst);
        });
        let start = Instant::now();
        assert!(matches!(conn.recv_msg::<Msg>(), Err(ConnError::Cancelled)));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn the_server_side_works_over_any_reader_and_writer() {
        // What `sync-serve` does over stdin and stdout: here, two pipes.
        let (to_server, server_in) = UnixStream::pair().unwrap();
        let (server_out, from_server) = UnixStream::pair().unwrap();
        let mut server = Conn::new(
            server_in,
            server_out,
            quick(),
            Arc::new(AtomicBool::new(false)),
        );
        let mut client = Conn::new(
            from_server,
            to_server,
            quick(),
            Arc::new(AtomicBool::new(false)),
        );
        client.send_msg(&Msg::Bye).unwrap();
        assert_eq!(server.recv_msg::<Msg>().unwrap(), Msg::Bye);
        server.send_msg(&Msg::Hello { version: 2 }).unwrap();
        assert_eq!(
            client.recv_hello_msg::<Msg>().unwrap(),
            Msg::Hello { version: 2 }
        );
    }
}
