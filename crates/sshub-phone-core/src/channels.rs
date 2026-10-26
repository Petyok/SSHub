//! Channels: the same frames (kinds 1 to 10) over a looping QR animation, a camera, a sound and Bluetooth LE. Port of
//! `assets/companion/src/channels.mjs` and `scan.mjs`. Rust never touches the camera, the screen or the speaker: it
//! turns bytes into module matrices and PCM, and gray camera frames into documents.

use qrcode::{Color, EcLevel, QrCode};
use sshub_sync_proto::fountain::{fountain_encode, FountainDecoder, Push, FRAME_HEADER};
use sshub_sync_proto::media::fsk;
use sshub_sync_proto::media::qrscan::{Gray, QrReader};

use crate::error::{CoreError, Result};
use crate::views::{AudioProfile, QrFrame, ScanEvent};

/// Reply QR frame size: the byte capacity of QR version 6, level M (106 B), so a 68 mm phone screen gives modules of
/// 1.4 mm, the size a fixed-focus laptop webcam read in the over-the-air test. The fountain format is unchanged (the
/// decoder takes the block size from the frame length); only the number of frames grows.
pub const QR_FRAME_BYTES: usize = 106;
pub const QR_FPS: u32 = 4;
pub const QR_PARITY_PERCENT: u32 = 25;

/// One QR code (byte mode, error correction M, smallest version that fits) as a module matrix.
pub fn qr_matrix(bytes: &[u8]) -> Result<QrFrame> {
    let code = QrCode::with_error_correction_level(bytes, EcLevel::M).map_err(|e| CoreError::internal(format!("qr: {e}")))?;
    let size = code.width();
    let modules = code.to_colors().into_iter().map(|c| u8::from(c == Color::Dark)).collect();
    Ok(QrFrame { size: size as u32, modules })
}

/// The frames of one lap of a document (fountain format, same as the terminal's challenge) as QR matrices: all data
/// frames, then the parity frames.
pub fn qr_frames(payload: &[u8], parity_percent: u32) -> Result<Vec<QrFrame>> {
    let frames = fountain_encode(payload, QR_FRAME_BYTES, parity_percent).map_err(|e| CoreError::bad(format!("{e:#}")))?;
    frames.iter().map(|f| qr_matrix(f)).collect()
}

// ---------------------------------------------------------------- audio

/// The PCM the speaker plays for a reply frame (`responseBurst` of the page): `LEN || frame || CRC16` as an MFSK burst,
/// 16 bit mono at 48 kHz (the f32 samples of the proto encoder times 32767, rounded).
pub fn audio_burst(frame: &[u8], profile: AudioProfile) -> Result<Vec<i16>> {
    if frame.len() > 255 {
        return Err(CoreError::bad("audio frame: payload over 255 bytes"));
    }
    let profile = match profile {
        AudioProfile::Fast => fsk::Profile::Fast,
        AudioProfile::Slow => fsk::Profile::Slow,
    };
    Ok(fsk::encode(frame, profile).into_iter().map(|v| (v * 32767.0).round() as i16).collect())
}

// ---------------------------------------------------------------- camera

/// Consecutive frames of one other document after which the collector follows it (or one lap of it, if shorter).
pub const SWITCH_AFTER_FRAMES: u32 = 6;

/// The other document a run of foreign frames belongs to: `(id, total, groups, block)` and how many arrived in a row.
struct Other {
    key: (u8, u8, usize, u8, usize),
    count: u32,
}

/// `DocumentCollector` of `scan.mjs`: fountain frames to one document. A stray frame of another document is ignored; when
/// the same other document keeps arriving with no frame of the current one in between, the partial document is dropped
/// and the new one is followed.
struct Collector {
    dec: FountainDecoder,
    errors: u32,
    other: Option<Other>,
    /// Header of the document delivered last: the screen loops it, and its frames must not start a half collected copy.
    finished: Option<[u8; 4]>,
}

impl Collector {
    fn new() -> Self {
        Self { dec: FountainDecoder::new(), errors: 0, other: None, finished: None }
    }

    /// The document bytes once complete.
    fn push(&mut self, frame: &[u8]) -> Option<Vec<u8>> {
        if let (Some(done), Some(key)) = (self.finished, frame.get(1..5)) {
            if key == done {
                return None;
            }
            self.finished = None;
        }
        match self.dec.push(frame) {
            Ok(Push::OtherDocument) => return self.foreign(frame),
            Ok(_) => {
                self.other = None;
                match self.dec.finish() {
                    Ok(Some(doc)) => {
                        self.dec = FountainDecoder::new();
                        self.finished = frame.get(1..5).and_then(|k| k.try_into().ok());
                        return Some(doc);
                    }
                    Ok(None) => {}
                    Err(_) => self.damaged(),
                }
            }
            Err(_) => self.damaged(),
        }
        None
    }

    fn damaged(&mut self) {
        self.errors += 1;
        self.dec = FountainDecoder::new();
        self.other = None;
    }

    fn foreign(&mut self, frame: &[u8]) -> Option<Vec<u8>> {
        // the decoder parsed the header already: magic, id(2), total u16, groups, seq u16
        let total = (usize::from(frame[3]) << 8) | usize::from(frame[4]);
        let block = frame.len() - FRAME_HEADER;
        let lap = total.div_ceil(block) + usize::from(frame[5]);
        let key = (frame[1], frame[2], total, frame[5], block);
        if self.other.as_ref().is_some_and(|o| o.key == key) {
            if let Some(o) = self.other.as_mut() {
                o.count += 1;
            }
        } else {
            self.other = Some(Other { key, count: 1 });
        }
        let count = self.other.as_ref().map_or(0, |o| o.count);
        if (count as usize) < lap.min(SWITCH_AFTER_FRAMES as usize) {
            return None;
        }
        self.dec = FountainDecoder::new();
        self.other = None;
        self.push(frame)
    }

    fn progress(&self) -> (u32, u32) {
        self.dec.progress().map_or((0, 0), |(h, t)| (h as u32, t as u32))
    }
}

/// Gray camera frames to documents: a QR reader, then the collector. One scanner per scan screen.
pub struct Scanner {
    reader: QrReader,
    collector: Collector,
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner {
    pub fn new() -> Self {
        Self { reader: QrReader::default(), collector: Collector::new() }
    }

    /// Starts a new scan (also forgets the document delivered last, so it can be scanned again).
    pub fn reset(&mut self) {
        self.collector = Collector::new();
    }

    /// Every QR code read in the image goes to the collector (frames not longer than the fountain header are noise, as in
    /// the page). Events: a `Document` per completed document, then one `Progress` with the state after the image.
    pub fn push_gray(&mut self, w: u32, h: u32, luma: &[u8]) -> Vec<ScanEvent> {
        let (width, height) = (w as usize, h as usize);
        if width == 0 || height == 0 || luma.len() < width * height {
            return Vec::new();
        }
        let mut events = Vec::new();
        for payload in self.reader.read(Gray { width, height, pixels: luma }) {
            if payload.len() <= FRAME_HEADER {
                continue;
            }
            if let Some(bytes) = self.collector.push(&payload) {
                events.push(ScanEvent::Document { bytes });
            }
        }
        let (have, total) = self.collector.progress();
        if total > 0 && !events.iter().any(|e| matches!(e, ScanEvent::Document { .. })) {
            events.push(ScanEvent::Progress { have, total, damaged: self.collector.errors });
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_becomes_a_looping_set_of_square_matrices() {
        let doc = vec![7u8; 1000];
        let frames = qr_frames(&doc, QR_PARITY_PERCENT).unwrap();
        // 1000 bytes / (106 - 8 header) = 11 data frames + 25 % parity groups
        assert!(frames.len() > 11);
        for f in &frames {
            assert_eq!(f.modules.len() as u32, f.size * f.size);
            assert!(f.modules.iter().all(|m| *m <= 1));
        }
        assert!(qr_frames(&[], 25).is_err());
    }

    // ---- audio

    #[test]
    fn the_burst_is_the_page_burst_as_i16() {
        let frame = vec![0x5a; 128];
        let pcm = audio_burst(&frame, AudioProfile::Fast).unwrap();
        let f = fsk::encode(&frame, fsk::Profile::Fast);
        assert_eq!(pcm.len(), f.len());
        assert_eq!(pcm[1000], (f[1000] * 32767.0).round() as i16);
        assert!(pcm.iter().map(|v| i32::from(*v).abs()).max().unwrap() <= 16384);
        let back: Vec<f32> = pcm.iter().map(|v| f32::from(*v) / 32767.0).collect();
        assert_eq!(fsk::decode_all(&back, 48_000), vec![frame.clone()]);
        let slow = audio_burst(&frame, AudioProfile::Slow).unwrap();
        assert!(slow.len() > pcm.len());
        assert!(audio_burst(&[0; 256], AudioProfile::Fast).is_err());
    }

    // ---- scanner

    /// A QR matrix as a gray image: four module quiet zone, `scale` pixels per module.
    fn gray(f: &QrFrame, scale: usize) -> (u32, u32, Vec<u8>) {
        let n = f.size as usize;
        let side = (n + 8) * scale;
        let mut px = vec![255u8; side * side];
        for r in 0..n {
            for c in 0..n {
                if f.modules[r * n + c] == 1 {
                    for y in 0..scale {
                        for x in 0..scale {
                            px[((r + 4) * scale + y) * side + (c + 4) * scale + x] = 0;
                        }
                    }
                }
            }
        }
        (side as u32, side as u32, px)
    }

    fn doc(seed: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| (i as u8).wrapping_mul(13).wrapping_add(seed)).collect()
    }

    fn feed(s: &mut Scanner, f: &QrFrame) -> Vec<ScanEvent> {
        let (w, h, px) = gray(f, 6);
        s.push_gray(w, h, &px)
    }

    #[test]
    fn a_qr_animation_becomes_the_document() {
        let d = doc(1, 700);
        let frames = qr_frames(&d, QR_PARITY_PERCENT).unwrap();
        let mut s = Scanner::new();
        let mut got = None;
        for (i, f) in frames.iter().enumerate() {
            if i == 2 {
                continue; // a missed frame: the parity group repairs it or the next lap brings it
            }
            for e in feed(&mut s, f) {
                match e {
                    ScanEvent::Document { bytes } => got = Some(bytes),
                    ScanEvent::Progress { have, total, damaged } => {
                        assert!(have <= total && total > 0 && damaged == 0);
                    }
                }
            }
        }
        if got.is_none() {
            for e in feed(&mut s, &frames[2]) {
                if let ScanEvent::Document { bytes } = e {
                    got = Some(bytes);
                }
            }
        }
        assert_eq!(got.unwrap(), d);
        // the screen keeps looping the same document: it is not delivered twice, until reset
        assert!(feed(&mut s, &frames[0]).is_empty());
        s.reset();
        assert!(matches!(feed(&mut s, &frames[0]).as_slice(), [ScanEvent::Progress { .. }]));
        // an empty or blank image says nothing while no document is being collected
        s.reset();
        assert!(s.push_gray(0, 0, &[]).is_empty());
        assert!(s.push_gray(10, 10, &[255; 100]).is_empty());
    }

    #[test]
    fn six_frames_of_another_document_switch_to_it() {
        let a = qr_frames(&doc(1, 700), QR_PARITY_PERCENT).unwrap();
        let bdoc = doc(9, 500);
        let b = qr_frames(&bdoc, QR_PARITY_PERCENT).unwrap();
        let mut s = Scanner::new();
        feed(&mut s, &a[0]);
        feed(&mut s, &a[1]);
        // five stray frames of B do not switch, a frame of A in between resets the count
        for f in &b[..5] {
            let e = feed(&mut s, f);
            assert!(e.iter().all(|e| !matches!(e, ScanEvent::Document { .. })));
        }
        feed(&mut s, &a[2]);
        let mut got = None;
        for round in 0..2 {
            for f in &b {
                for e in feed(&mut s, f) {
                    if let ScanEvent::Document { bytes } = e {
                        got = Some(bytes);
                    }
                }
            }
            if got.is_some() {
                break;
            }
            assert!(round == 0);
        }
        assert_eq!(got.unwrap(), bdoc, "the collector followed B after six consecutive frames");
    }

    #[test]
    fn the_foreign_rule_counts_only_unbroken_runs_and_short_laps() {
        let a = fountain_encode(&doc(1, 700), QR_FRAME_BYTES, 25).unwrap();
        let b = fountain_encode(&doc(9, 500), QR_FRAME_BYTES, 25).unwrap();
        let mut c = Collector::new();
        c.push(&a[0]);
        for f in &b[..5] {
            assert!(c.push(f).is_none());
        }
        assert_eq!(c.progress().1, 8, "still collecting A (700 / 98 = 8 blocks)");
        c.push(&a[1]); // a frame of the current document breaks the run
        for f in &b[..5] {
            assert!(c.push(f).is_none());
        }
        assert_eq!(c.progress().1, 8);
        c.push(&b[5]);
        assert_eq!(c.progress().1, 6, "the sixth consecutive frame switched to B (500 / 98 = 6 blocks)");
        // a document of one frame is a lap shorter than six: one frame switches
        let tiny = fountain_encode(&doc(3, 40), QR_FRAME_BYTES, 25).unwrap();
        let mut c = Collector::new();
        c.push(&a[0]);
        assert_eq!(c.push(&tiny[0]).unwrap(), doc(3, 40));
        // a damaged frame resets the decoder and is counted
        let mut c = Collector::new();
        c.push(&a[0]);
        let mut bad = a[1].clone();
        bad[6] = 0xff;
        bad[7] = 0xff;
        c.push(&bad);
        assert_eq!(c.errors, 1);
        assert_eq!(c.progress(), (0, 0));
    }
}
