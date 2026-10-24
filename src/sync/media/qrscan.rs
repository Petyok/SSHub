//! Receiver of the phone's reverse QR: decodes QR codes from grayscale camera frames and feeds
//! their payloads to a [`FountainDecoder`] until the reply document is complete. No camera here
//! (`cam.rs` supplies frames), so this builds and is tested without any capture crate.
//!
//! The phone shows the reply as looping fountain frames, black on white (companion
//! `channels.mjs`): see `fountain.rs` for the frame format. A frame missed in one lap arrives in
//! the next; duplicates and out-of-order frames are harmless.

use crate::sync::fountain::{FountainDecoder, Push};

/// A borrowed 8-bit grayscale image, row-major, `pixels.len() == width * height`.
#[derive(Clone, Copy)]
pub struct Gray<'a> {
    pub width: usize,
    pub height: usize,
    pub pixels: &'a [u8],
}

/// Luma of packed RGB or BGR-less `rgb` (3 bytes per pixel, integer BT.601).
pub fn gray_from_rgb(rgb: &[u8]) -> Vec<u8> {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .map(|p| ((u32::from(p[0]) * 77 + u32::from(p[1]) * 150 + u32::from(p[2]) * 29) >> 8) as u8)
        .collect()
}

/// Luma plane of packed YUYV (4:2:2): every other byte.
pub fn gray_from_yuyv(yuyv: &[u8]) -> Vec<u8> {
    yuyv.iter().step_by(2).copied().collect()
}

/// Largest image the 2x retry is attempted on (a 1600x900 frame becomes 3200x1800).
const RETRY_MAX_PIXELS: usize = 1_600 * 900;

/// 2x bilinear upscale (quirc samples modules more reliably when a module spans more pixels).
fn upscale2(img: Gray) -> Vec<u8> {
    let (w, h) = (img.width, img.height);
    let px = |x: usize, y: usize| f32::from(img.pixels[y * w + x]);
    let mut out = Vec::with_capacity(4 * w * h);
    for y in 0..2 * h {
        let fy = ((y as f32 + 0.5) / 2.0 - 0.5).max(0.0);
        let (y0, ay) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for x in 0..2 * w {
            let fx = ((x as f32 + 0.5) / 2.0 - 0.5).max(0.0);
            let (x0, ax) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(w - 1);
            let top = px(x0, y0) * (1.0 - ax) + px(x1, y0) * ax;
            let bottom = px(x0, y1) * (1.0 - ax) + px(x1, y1) * ax;
            out.push((top * (1.0 - ay) + bottom * ay) as u8);
        }
    }
    out
}

/// QR reader over grayscale frames; keeps quirc's work buffers between frames.
#[derive(Default)]
pub struct QrReader {
    quirc: quircs::Quirc,
}

impl QrReader {
    /// Payloads and the number of finder-pattern grids located (decoded or not).
    fn pass(&mut self, width: usize, height: usize, pixels: &[u8]) -> (Vec<Vec<u8>>, usize) {
        let mut grids = 0;
        let mut found = Vec::new();
        for code in self.quirc.identify(width, height, pixels).flatten() {
            grids += 1;
            if let Ok(data) = code.decode() {
                found.push(data.payload);
            }
        }
        (found, grids)
    }

    /// Payloads of every QR code found in the image (usually zero or one). When a code is
    /// located but does not decode (blur, low resolution) the image is retried once at 2x:
    /// measured on camera-like variants this decodes about 40% more frames than the plain
    /// pass, and it costs anything only on frames that already hold a code.
    pub fn read(&mut self, img: Gray) -> Vec<Vec<u8>> {
        if img.width == 0 || img.pixels.len() < img.width * img.height {
            return Vec::new();
        }
        let (found, grids) = self.pass(img.width, img.height, img.pixels);
        if found.is_empty() && grids > 0 && img.width * img.height <= RETRY_MAX_PIXELS {
            return self.pass(2 * img.width, 2 * img.height, &upscale2(img)).0;
        }
        found
    }
}

/// One-shot form of [`QrReader::read`].
pub fn decode_qr(img: Gray) -> Vec<Vec<u8>> {
    QrReader::default().read(img)
}

/// Frames of another document in a row before the collector drops what it holds and follows
/// the new one (a stray fountain document, e.g. the challenge, must not block the reply).
const FOREIGN_RESET: u32 = 24;

/// What one camera frame did to the collection.
#[derive(Debug, PartialEq, Eq)]
pub enum Scan {
    Nothing,
    /// A new block arrived: `(held, needed)`.
    Progress(usize, usize),
    /// The whole document. The collector starts over; the caller decides whether it is the
    /// document it wanted.
    Complete(Vec<u8>),
}

#[derive(Default)]
pub struct ReplyCollector {
    reader: QrReader,
    dec: FountainDecoder,
    foreign: u32,
}

impl ReplyCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_image(&mut self, img: Gray) -> Scan {
        let mut last = Scan::Nothing;
        for frame in self.reader.read(img) {
            match self.push_frame(&frame) {
                Scan::Nothing => {}
                s => last = s,
            }
        }
        last
    }

    /// One decoded QR payload.
    pub fn push_frame(&mut self, frame: &[u8]) -> Scan {
        let pushed = match self.dec.push(frame) {
            Ok(Push::OtherDocument) => {
                self.foreign += 1;
                if self.foreign < FOREIGN_RESET {
                    return Scan::Nothing;
                }
                self.reset();
                match self.dec.push(frame) {
                    Ok(p) => p,
                    Err(_) => return Scan::Nothing,
                }
            }
            Ok(p) => {
                self.foreign = 0;
                p
            }
            Err(_) => return Scan::Nothing,
        };
        match self.dec.finish() {
            Ok(Some(doc)) => {
                self.reset();
                Scan::Complete(doc)
            }
            Err(_) => {
                self.reset();
                Scan::Nothing
            }
            Ok(None) if pushed == Push::Progress => match self.dec.progress() {
                Some((held, needed)) => Scan::Progress(held, needed),
                None => Scan::Nothing,
            },
            Ok(None) => Scan::Nothing,
        }
    }

    fn reset(&mut self) {
        self.dec = FountainDecoder::new();
        self.foreign = 0;
    }
}
