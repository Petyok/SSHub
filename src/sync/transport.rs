//! Length-prefixed JSON frames: a `u32` big-endian length, then a UTF-8 JSON
//! body. Ported from the measured PoC F rig.
//!
//! Every cap is enforced before the cost it guards: the length prefix is
//! checked against [`limits::MAX_FRAME_BYTES`] before the body is allocated,
//! and the nesting depth against [`limits::MAX_JSON_DEPTH`] by a byte scan that
//! runs before `serde_json` sees the body.

use std::fmt;
use std::io::{self, Read, Write};

use super::limits::{MAX_FRAME_BYTES, MAX_JSON_DEPTH};

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
                write!(f, "length prefix {n} exceeds the {MAX_FRAME_BYTES} byte frame cap")
            }
            FrameError::Empty => write!(f, "zero-length frame"),
            FrameError::Truncated => write!(f, "stream ended inside a frame"),
            FrameError::Depth(d) => {
                write!(f, "JSON nesting depth {d} exceeds the cap of {MAX_JSON_DEPTH}")
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
            ("control: nested arrays depth 32 (accepted)", nested(32), true),
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
        assert!(check_depth(&vec![b'['; MAX_JSON_DEPTH]).is_ok());
        assert!(matches!(
            check_depth(&vec![b'['; MAX_JSON_DEPTH + 1]),
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
        assert!(read_frame(&mut r).unwrap().is_none(), "clean EOF at a boundary");
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
