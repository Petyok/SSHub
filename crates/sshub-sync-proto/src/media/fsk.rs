//! Receiver of the phone's audio reply: a pure decoder for the MFSK burst of the companion page
//! (`channels.mjs` / PoC H `share-lib.js`). No audio device here, so it builds and is tested
//! without `cpal`; `mic.rs` only feeds it samples.
//!
//! Burst layout, all at 48 kHz: a 300 ms sync tone (1000 Hz), then 4-bit symbols on 16 tones
//! (1600 Hz + 100 Hz * nibble), high nibble first, one symbol per 30 ms (fast profile) or 60 ms
//! (slow profile), 3 or 5 ms raised-cosine edges. Payload frame = `LEN(1) || payload ||
//! CRC16-CCITT-FALSE(2)` over `LEN || payload`. The profile is not announced: both are tried and
//! the CRC decides.
//!
//! The decoder is streaming. Input at any rate is resampled to 48 kHz (linear interpolation, a
//! moving average first when downsampling by 2 or more); the 1000 Hz tone fraction of a sliding
//! 40 ms window finds a burst; symbol timing is aligned by maximising tone energy over the first
//! eight symbols; every symbol is a Goertzel argmax over the 16 tones.
//!
//! ponytail: no clock-drift tracking after alignment (a 100 ppm sound card moves 1.6 ms over the
//! longest slow burst, far below the 30 ms symbol); add per-symbol early/late gating if a real
//! device ever shows drift.

use std::f64::consts::PI;

/// Sample rate the burst is defined at.
pub const RATE: usize = 48_000;
const BASE_HZ: f64 = 1600.0;
const STEP_HZ: f64 = 100.0;
const TONES: usize = 16;
const SYNC_HZ: f64 = 1000.0;
const PREAMBLE: usize = RATE * 300 / 1000;
const HOP: usize = RATE / 100;
const WIN: usize = HOP * 4;
/// Tone fraction (share of the window energy sitting at 1000 Hz) that counts as sync. Pure noise
/// stays near `2 / WIN`; a window half full of sync tone reads 0.5 and 0 dB SNR halves that.
const SYNC_FRACTION: f64 = 0.10;
/// Consecutive hops above the threshold before a burst is assumed (the real preamble gives 27).
const SYNC_RUN: usize = 16;
const ALIGN_SYMBOLS: usize = 8;
/// Mean share of a symbol window's energy in its strongest tone; wrong symbol length or noise
/// gives about 0.5 or 1/16.
const PURITY: f64 = 0.4;
/// Largest burst: 255 byte payload + 3, two symbols per byte, slow.
const MAX_BUF: usize = PREAMBLE + 2 * 258 * RATE * 60 / 1000 + RATE * 2;

/// Symbol length of one transmit profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Fast,
    Slow,
}

impl Profile {
    const ALL: [Profile; 2] = [Profile::Fast, Profile::Slow];

    fn symbol(self) -> usize {
        match self {
            Profile::Fast => RATE * 30 / 1000,
            Profile::Slow => RATE * 60 / 1000,
        }
    }
}

/// CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF); check value of "123456789" is 0x29B1.
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Power of `hz` over `x` (Goertzel), `|X|^2` of the DFT at that frequency.
fn goertzel(x: &[f32], hz: f64) -> f64 {
    let coeff = 2.0 * (2.0 * PI * hz / RATE as f64).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = f64::from(v) + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/// Share of the window's energy that is the sync tone: 1 for a pure tone, the covered fraction
/// for a partly covered window, about `2 / len` for noise.
fn sync_fraction(x: &[f32]) -> f64 {
    let energy: f64 = x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum();
    if energy < 1e-12 {
        return 0.0;
    }
    2.0 * goertzel(x, SYNC_HZ) / (x.len() as f64 * energy)
}

fn tone_powers(x: &[f32]) -> [f64; TONES] {
    std::array::from_fn(|t| goertzel(x, BASE_HZ + STEP_HZ * t as f64))
}

fn argmax(p: &[f64; TONES]) -> usize {
    p.iter()
        .enumerate()
        .fold(
            (0, f64::MIN),
            |a, (i, v)| if *v > a.1 { (i, *v) } else { a },
        )
        .0
}

/// Streaming linear resampler to [`RATE`].
struct Resampler {
    ratio: f64,
    /// Window of the pre-filter (moving average) in input samples.
    avg: usize,
    avg_tail: Vec<f32>,
    pos: f64,
    tail: Vec<f32>,
}

impl Resampler {
    fn new(input_rate: u32) -> Self {
        let ratio = f64::from(input_rate) / RATE as f64;
        Resampler {
            ratio,
            avg: if ratio >= 2.0 { ratio as usize } else { 1 },
            avg_tail: Vec::new(),
            pos: 0.0,
            tail: Vec::new(),
        }
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.ratio == 1.0 {
            out.extend_from_slice(input);
            return;
        }
        if self.avg > 1 {
            self.avg_tail.extend_from_slice(input);
            let n = self.avg_tail.len();
            let k = self.avg;
            let start = k - 1;
            if n >= k {
                let mut sum: f32 = self.avg_tail[..k - 1].iter().sum();
                for i in start..n {
                    sum += self.avg_tail[i];
                    self.tail.push(sum / k as f32);
                    sum -= self.avg_tail[i + 1 - k];
                }
            }
            let keep = n.min(k - 1);
            self.avg_tail.drain(..n - keep);
        } else {
            self.tail.extend_from_slice(input);
        }
        while self.pos + 1.0 < self.tail.len() as f64 {
            let i = self.pos as usize;
            let f = (self.pos - i as f64) as f32;
            out.push(self.tail[i] * (1.0 - f) + self.tail[i + 1] * f);
            self.pos += self.ratio;
        }
        let drop = (self.pos as usize).min(self.tail.len());
        self.tail.drain(..drop);
        self.pos -= drop as f64;
    }
}

enum Slot {
    /// Waiting for samples (until `need`) to align and judge this profile.
    Pending {
        need: usize,
    },
    /// Aligned; waiting for the whole frame (until `need`).
    Aligned {
        start: usize,
        need: usize,
    },
    Dead,
}

struct Burst {
    onset: usize,
    slots: [Slot; 2],
}

enum Attempt {
    Done(Vec<u8>, usize),
    Wait,
    Dead,
}

/// Streaming decoder: feed PCM chunks, collect the payloads of complete, CRC-valid bursts.
pub struct FskDecoder {
    rs: Resampler,
    buf: Vec<f32>,
    scan: usize,
    run: usize,
    run_start: usize,
    burst: Option<Burst>,
}

impl FskDecoder {
    pub fn new(input_rate: u32) -> Self {
        FskDecoder {
            rs: Resampler::new(input_rate),
            buf: Vec::new(),
            scan: 0,
            run: 0,
            run_start: 0,
            burst: None,
        }
    }

    /// True from the moment a sync tone is recognised until the burst is decoded or given up.
    pub fn receiving(&self) -> bool {
        self.burst.is_some()
    }

    /// Mono samples in `-1..1` at the rate given to [`FskDecoder::new`].
    pub fn push(&mut self, pcm: &[f32]) -> Vec<Vec<u8>> {
        self.rs.process(pcm, &mut self.buf);
        let mut done = Vec::new();
        loop {
            match self.burst.take() {
                None => {
                    if !self.find_burst() {
                        break;
                    }
                }
                Some(mut b) => {
                    if let Some((payload, end)) = self.advance(&mut b) {
                        done.push(payload);
                        self.resume_at(end);
                    } else if b.slots.iter().all(|s| matches!(s, Slot::Dead)) {
                        self.resume_at(b.onset + PREAMBLE);
                    } else {
                        self.burst = Some(b);
                        break;
                    }
                }
            }
        }
        self.trim();
        done
    }

    fn resume_at(&mut self, at: usize) {
        self.scan = at;
        self.run = 0;
    }

    /// Slides the sync detector over unseen samples. True when a burst was found.
    fn find_burst(&mut self) -> bool {
        while self.scan + WIN <= self.buf.len() {
            if sync_fraction(&self.buf[self.scan..self.scan + WIN]) >= SYNC_FRACTION {
                if self.run == 0 {
                    self.run_start = self.scan;
                }
                self.run += 1;
                if self.run >= SYNC_RUN {
                    self.burst = Some(Burst {
                        onset: self.run_start,
                        slots: [Slot::Pending { need: 0 }, Slot::Pending { need: 0 }],
                    });
                    self.run = 0;
                    self.scan = self.run_start + SYNC_RUN * HOP;
                    return true;
                }
            } else {
                self.run = 0;
            }
            self.scan += HOP;
        }
        false
    }

    /// Runs both profiles as far as the buffered samples allow.
    fn advance(&self, b: &mut Burst) -> Option<(Vec<u8>, usize)> {
        for (slot, profile) in b.slots.iter_mut().zip(Profile::ALL) {
            match self.attempt(b.onset, profile, slot) {
                Attempt::Done(p, end) => return Some((p, end)),
                Attempt::Dead => *slot = Slot::Dead,
                Attempt::Wait => {}
            }
        }
        None
    }

    fn attempt(&self, onset: usize, profile: Profile, slot: &mut Slot) -> Attempt {
        let n = profile.symbol();
        let buf = &self.buf;
        if let Slot::Pending { need } | Slot::Aligned { need, .. } = slot {
            if buf.len() < *need {
                return Attempt::Wait;
            }
        }
        let start = match *slot {
            Slot::Dead => return Attempt::Dead,
            Slot::Aligned { start, .. } => start,
            Slot::Pending { .. } => {
                // the preamble ends within [onset + P - 8 ms, onset + P + 32 ms]
                let guess = onset + PREAMBLE + RATE * 12 / 1000;
                let span = RATE * 28 / 1000;
                let need = guess + span + ALIGN_SYMBOLS * n;
                if buf.len() < need {
                    *slot = Slot::Pending { need };
                    return Attempt::Wait;
                }
                let energy = |s: usize| -> f64 {
                    (0..ALIGN_SYMBOLS)
                        .map(|i| {
                            let p = tone_powers(&buf[s + i * n..s + (i + 1) * n]);
                            p[argmax(&p)]
                        })
                        .sum()
                };
                let ms = RATE / 1000;
                let mut best = (guess - span, f64::MIN);
                for s in ((guess - span)..=(guess + span)).step_by(ms) {
                    let e = energy(s);
                    if e > best.1 {
                        best = (s, e);
                    }
                }
                let fine = best.0;
                for s in (fine.saturating_sub(ms)..=fine + ms).step_by(ms / 4) {
                    if s + ALIGN_SYMBOLS * n > buf.len() {
                        break;
                    }
                    let e = energy(s);
                    if e > best.1 {
                        best = (s, e);
                    }
                }
                let purity: f64 = (0..ALIGN_SYMBOLS)
                    .map(|i| {
                        let p = tone_powers(&buf[best.0 + i * n..best.0 + (i + 1) * n]);
                        p[argmax(&p)] / p.iter().sum::<f64>().max(1e-30)
                    })
                    .sum::<f64>()
                    / ALIGN_SYMBOLS as f64;
                if purity < PURITY {
                    return Attempt::Dead;
                }
                best.0
            }
        };
        let nibble = |i: usize| -> usize {
            let off = start + i * n;
            argmax(&tone_powers(&buf[off..off + n]))
        };
        let len = (nibble(0) << 4) | nibble(1);
        if len == 0 {
            return Attempt::Dead;
        }
        let symbols = (len + 3) * 2;
        let end = start + symbols * n;
        if buf.len() < end {
            *slot = Slot::Aligned { start, need: end };
            return Attempt::Wait;
        }
        let bytes: Vec<u8> = (0..len + 3)
            .map(|i| ((nibble(2 * i) << 4) | nibble(2 * i + 1)) as u8)
            .collect();
        let crc = u16::from_be_bytes([bytes[len + 1], bytes[len + 2]]);
        if crc16(&bytes[..len + 1]) != crc {
            return Attempt::Dead;
        }
        Attempt::Done(bytes[1..=len].to_vec(), end)
    }

    /// Drops samples that can no longer matter and keeps the buffer bounded.
    fn trim(&mut self) {
        let keep_from = match &self.burst {
            Some(b) => b.onset,
            None if self.run > 0 => self.run_start,
            None => self.scan,
        }
        .min(self.buf.len());
        let overflow = self.buf.len().saturating_sub(MAX_BUF);
        let drop = if keep_from >= RATE { keep_from } else { 0 }.max(overflow);
        if drop == 0 {
            return;
        }
        self.buf.drain(..drop);
        self.scan = self.scan.saturating_sub(drop);
        self.run_start = self.run_start.saturating_sub(drop);
        if let Some(b) = &mut self.burst {
            if b.onset < drop {
                self.burst = None;
                self.run = 0;
            } else {
                b.onset -= drop;
                for s in &mut b.slots {
                    *s = Slot::Pending { need: 0 };
                }
            }
        }
    }
}

/// All payloads in one finished recording.
pub fn decode_all(pcm: &[f32], input_rate: u32) -> Vec<Vec<u8>> {
    let mut d = FskDecoder::new(input_rate);
    d.push(pcm)
}

/// The burst the page plays (`channels.mjs` `fskEncode` over `audioFrame`): a 300 ms 1000 Hz sync tone, then
/// `LEN(1) || payload || CRC16(2)` as 4-bit symbols, high nibble first, on the 16 tones, at 48 kHz mono, amplitude 0.5.
/// Every symbol restarts its phase at zero and has raised-cosine edges of 3 ms (fast) or 5 ms (slow), the preamble
/// uses the same edge. Samples are computed in f64 and stored as f32 like the page's `Float32Array`.
///
/// # Panics
/// When `payload` is longer than 255 bytes (the length byte); callers check first.
pub fn encode(payload: &[u8], profile: Profile) -> Vec<f32> {
    assert!(payload.len() <= 255, "audio frame: payload over 255 bytes");
    let n = profile.symbol();
    let ramp = match profile {
        Profile::Fast => RATE * 3 / 1000,
        Profile::Slow => RATE * 5 / 1000,
    };
    let mut framed = Vec::with_capacity(payload.len() + 3);
    framed.push(payload.len() as u8);
    framed.extend_from_slice(payload);
    let crc = crc16(&framed);
    framed.extend_from_slice(&crc.to_be_bytes());

    let mut out = Vec::with_capacity(PREAMBLE + framed.len() * 2 * n);
    tone(&mut out, SYNC_HZ, PREAMBLE, ramp);
    for b in &framed {
        for nibble in [b >> 4, b & 0x0f] {
            tone(&mut out, BASE_HZ + STEP_HZ * f64::from(nibble), n, ramp);
        }
    }
    out
}

/// One tone of `len` samples with raised-cosine edges of `ramp` samples (`toneInto` of the page).
fn tone(out: &mut Vec<f32>, hz: f64, len: usize, ramp: usize) {
    let w = 2.0 * PI * hz / RATE as f64;
    for i in 0..len {
        let env = if i < ramp {
            0.5 * (1.0 - (PI * i as f64 / ramp as f64).cos())
        } else if i >= len - ramp {
            0.5 * (1.0 - (PI * (len - 1 - i) as f64 / ramp as f64).cos())
        } else {
            1.0
        };
        out.push((0.5 * env * (w * i as f64).sin()) as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Vec<u8> {
        (0..128u32).map(|i| (i * 37 + 11) as u8).collect()
    }

    #[test]
    fn the_decoder_reads_what_encode_makes_at_both_profiles() {
        for profile in [Profile::Fast, Profile::Slow] {
            let mut pcm = vec![0.0f32; RATE / 4];
            pcm.extend(encode(&payload(), profile));
            pcm.extend(vec![0.0f32; RATE / 4]);
            assert_eq!(
                decode_all(&pcm, RATE as u32),
                vec![payload()],
                "{profile:?}"
            );
        }
    }

    #[test]
    fn lengths_follow_the_page() {
        // 300 ms preamble + (1 + 128 + 2) bytes * 2 symbols * 30 ms (fast) or 60 ms (slow)
        assert_eq!(
            encode(&payload(), Profile::Fast).len(),
            14_400 + 131 * 2 * 1_440
        );
        assert_eq!(
            encode(&payload(), Profile::Slow).len(),
            14_400 + 131 * 2 * 2_880
        );
        let pcm = encode(&[], Profile::Fast);
        assert_eq!(pcm.len(), 14_400 + 3 * 2 * 1_440);
        assert!(pcm.iter().all(|v| v.abs() <= 0.5));
        // the first sample of every symbol is silent (the ramp starts at zero)
        assert_eq!(pcm[0], 0.0);
        assert_eq!(pcm[14_400], 0.0);
    }

    #[test]
    #[should_panic(expected = "over 255")]
    fn a_long_payload_is_refused() {
        encode(&[0; 256], Profile::Fast);
    }
}
