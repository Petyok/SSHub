//! Decoder tests. Audio: PCM produced by the phone's own JS encoder (committed WAVs, see
//! `tests/fixtures/media/README`), degraded with noise and resampling by helpers that share no
//! code with the decoder. QR: the frames the page drew on its canvas (committed PBMs) and
//! synthetic loops from `fountain_encode` + the `qrcode` crate.
//!
//! `cargo test sync::media -- --nocapture` prints the robustness tables.

use std::path::{Path, PathBuf};

use qrcode::{Color, EcLevel, QrCode};

use super::fsk::{crc16, decode_all, FskDecoder};
use super::qrscan::{decode_qr, Gray, ReplyCollector, Scan};
use super::{Emitter, MediaEvent, MediaKind};
use crate::sync::fountain::fountain_encode;
use crate::sync::wire::ShareResponseWire;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media")
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(name)).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

// ------------------------------------------------------------------ small deterministic rng

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        let (u, v) = (1.0 - self.unit(), self.unit());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

// ------------------------------------------------------------------ audio helpers

/// 16-bit mono PCM WAV (what `media-gen` wrote) to floats.
fn wav(name: &str) -> (Vec<f32>, u32) {
    let b = fixture(name);
    assert_eq!(&b[..4], b"RIFF");
    let rate = u32::from_le_bytes(b[24..28].try_into().unwrap());
    assert_eq!(&b[36..40], b"data");
    let pcm = b[44..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32767.0)
        .collect();
    (pcm, rate)
}

fn power(x: &[f32]) -> f64 {
    x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64
}

/// White noise at `snr_db` relative to the mean power of `reference`, added to all of `x`.
fn add_noise(x: &mut [f32], reference: &[f32], snr_db: f64, rng: &mut Rng) {
    let sigma = (power(reference) / 10f64.powf(snr_db / 10.0)).sqrt();
    for v in x {
        *v += (sigma * rng.gauss()) as f32;
    }
}

/// Windowed-sinc resampler (the analog front end of a sound card, not the decoder's).
fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    let ratio = f64::from(from) / f64::from(to);
    let cutoff = (1.0 / ratio).min(1.0) * 0.95;
    let half = 16.0 / cutoff.max(0.1);
    let n_out = (x.len() as f64 / ratio) as usize;
    (0..n_out)
        .map(|n| {
            let t = n as f64 * ratio;
            let lo = (t - half).ceil().max(0.0) as usize;
            let hi = ((t + half).floor() as usize).min(x.len() - 1);
            let mut acc = 0.0;
            let mut wsum = 0.0;
            for (i, v) in x.iter().enumerate().take(hi + 1).skip(lo) {
                let d = i as f64 - t;
                let arg = std::f64::consts::PI * d * cutoff;
                let sinc = if d == 0.0 { 1.0 } else { arg.sin() / arg };
                let w = 0.5 * (1.0 + (std::f64::consts::PI * d / half).cos());
                acc += f64::from(*v) * sinc * w;
                wsum += sinc * w;
            }
            (acc / wsum) as f32
        })
        .collect()
}

fn pad(burst: &[f32], lead: usize, tail: usize) -> Vec<f32> {
    let mut v = vec![0.0; lead];
    v.extend_from_slice(burst);
    v.extend(std::iter::repeat_n(0.0, tail));
    v
}

fn stream_decode(x: &[f32], rate: u32, chunk: usize) -> Vec<Vec<u8>> {
    let mut d = FskDecoder::new(rate);
    x.chunks(chunk).flat_map(|c| d.push(c)).collect()
}

fn short_reply() -> Vec<u8> {
    fixture("response-short.bin")
}

// ------------------------------------------------------------------ audio tests

#[test]
fn crc16_is_ccitt_false() {
    assert_eq!(crc16(b"123456789"), 0x29b1);
}

#[test]
fn phone_page_burst_decodes_to_the_exact_wire() {
    let (pcm, rate) = wav("fast-128.wav");
    assert_eq!(rate, 48_000);
    let got = decode_all(&pcm, rate);
    assert_eq!(got, vec![short_reply()]);
    let wire = ShareResponseWire::decode(&got[0]).unwrap();
    assert_eq!(wire.request_id, 4242);
}

#[test]
fn slow_profile_burst_decodes() {
    let (pcm, rate) = wav("slow-48.wav");
    assert_eq!(decode_all(&pcm, rate), vec![short_reply()[..48].to_vec()]);
}

#[test]
fn streaming_in_odd_chunks_with_silence_around() {
    let (fast, _) = wav("fast-128.wav");
    let (slow, _) = wav("slow-48.wav");
    for (burst, want) in [
        (&fast, short_reply()),
        (&slow, short_reply()[..48].to_vec()),
    ] {
        let x = pad(burst, 48_000 * 3 / 2, 48_000 * 2);
        for chunk in [441, 1024, 4096, 48_000] {
            assert_eq!(
                stream_decode(&x, 48_000, chunk),
                vec![want.clone()],
                "chunk {chunk}"
            );
        }
    }
}

#[test]
fn two_bursts_in_one_stream_both_profiles() {
    let (fast, _) = wav("fast-128.wav");
    let (slow, _) = wav("slow-48.wav");
    let mut x = pad(&slow, 20_000, 60_000);
    x.extend(pad(&fast, 0, 30_000));
    x.extend(pad(&slow, 5_000, 48_000));
    let got = stream_decode(&x, 48_000, 2048);
    assert_eq!(
        got,
        vec![
            short_reply()[..48].to_vec(),
            short_reply(),
            short_reply()[..48].to_vec()
        ]
    );
}

#[test]
fn silence_and_noise_yield_nothing() {
    let mut rng = Rng(7);
    let mut d = FskDecoder::new(48_000);
    let mut x = vec![0.0f32; 48_000 * 10];
    assert!(d.push(&x).is_empty());
    assert!(!d.receiving());
    for v in &mut x {
        *v = (0.1 * rng.gauss()) as f32;
    }
    assert!(d.push(&x).is_empty());
    assert!(!d.receiving());
}

#[test]
fn a_cut_off_burst_gives_nothing_and_does_not_panic() {
    let (fast, _) = wav("fast-128.wav");
    let cut = &fast[..fast.len() / 2];
    assert!(decode_all(&pad(cut, 10_000, 48_000), 48_000).is_empty());
}

#[test]
fn a_corrupted_burst_is_refused_by_the_crc() {
    let (mut fast, _) = wav("fast-128.wav");
    // wipe two symbols in the middle of the payload
    let at = 14_400 + 1440 * 100;
    fast[at..at + 2880].fill(0.0);
    assert!(decode_all(&pad(&fast, 10_000, 48_000), 48_000).is_empty());
}

/// Fixed seeds; one row per SNR (white noise relative to the burst power). Every seed must decode down to 0 dB; the lower rows are printed for the margin.
#[test]
fn noise_tolerance_both_profiles() {
    let (fast, _) = wav("fast-128.wav");
    let (slow, _) = wav("slow-48.wav");
    let seeds = 12u64;
    println!("snr_db   fast ok/{seeds}   slow ok/{seeds}");
    for snr in [20.0, 10.0, 5.0, 0.0, -3.0, -6.0, -9.0] {
        let mut ok = [0u64; 2];
        for seed in 1..=seeds {
            for (i, (burst, want)) in [
                (&fast, short_reply()),
                (&slow, short_reply()[..48].to_vec()),
            ]
            .into_iter()
            .enumerate()
            {
                let mut x = pad(burst, 40_000, 40_000);
                let mut rng = Rng(seed * 7919 + (snr + 20.0) as u64 + 1000);
                add_noise(&mut x, burst, snr, &mut rng);
                if stream_decode(&x, 48_000, 1920) == vec![want] {
                    ok[i] += 1;
                }
            }
        }
        println!("{snr:6}   {:9}   {:9}", ok[0], ok[1]);
        if snr >= 0.0 {
            assert_eq!(ok, [seeds, seeds], "all seeds must decode at {snr} dB");
        }
    }
}

#[test]
fn input_rates_other_than_48k() {
    let (fast, _) = wav("fast-128.wav");
    let (slow, _) = wav("slow-48.wav");
    println!("rate     fast  slow  fast+10dB  slow+10dB");
    for rate in [8_000u32, 16_000, 32_000, 44_100, 96_000, 192_000] {
        let mut res = [false; 4];
        for (i, (burst, want)) in [
            (&fast, short_reply()),
            (&slow, short_reply()[..48].to_vec()),
        ]
        .into_iter()
        .enumerate()
        {
            let clean = resample(&pad(burst, 30_000, 30_000), 48_000, rate);
            res[i] = stream_decode(&clean, rate, 1000) == vec![want.clone()];
            let mut noisy = clean.clone();
            add_noise(
                &mut noisy,
                &resample(burst, 48_000, rate),
                10.0,
                &mut Rng(99),
            );
            res[2 + i] = stream_decode(&noisy, rate, 1000) == vec![want];
        }
        println!(
            "{rate:6}   {:5} {:5} {:9} {:9}",
            res[0], res[1], res[2], res[3]
        );
        if rate >= 16_000 {
            assert!(res.iter().all(|r| *r), "{rate} Hz: {res:?}");
        }
    }
}

#[test]
fn sound_card_clock_offset() {
    let (fast, _) = wav("fast-128.wav");
    // a card whose clock runs ppm fast or slow: the same signal stretched, read as 48 kHz
    println!("clock offset ppm   fast burst");
    for ppm in [0i32, 100, -100, 500, -500, 1000, -1000, 2000, -2000] {
        let to = (1_000_000.0 * (1.0 + f64::from(ppm) / 1e6)) as u32;
        let x = resample(&pad(&fast, 30_000, 30_000), 1_000_000, to);
        let ok = stream_decode(&x, 48_000, 4096) == vec![short_reply()];
        println!("{ppm:16}   {ok}");
        if ppm.abs() <= 500 {
            assert!(ok, "{ppm} ppm");
        }
    }
}

// ------------------------------------------------------------------ image helpers

#[derive(Clone)]
struct Img {
    w: usize,
    h: usize,
    px: Vec<f32>,
}

impl Img {
    fn at(&self, x: isize, y: isize, edge: f32) -> f32 {
        if x < 0 || y < 0 || x >= self.w as isize || y >= self.h as isize {
            edge
        } else {
            self.px[y as usize * self.w + x as usize]
        }
    }

    fn bilinear(&self, x: f64, y: f64, edge: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let (xi, yi) = (x0 as isize, y0 as isize);
        let a = self.at(xi, yi, edge) * (1.0 - fx) + self.at(xi + 1, yi, edge) * fx;
        let b = self.at(xi, yi + 1, edge) * (1.0 - fx) + self.at(xi + 1, yi + 1, edge) * fx;
        a * (1.0 - fy) + b * fy
    }

    /// Area-average scaling (camera sensor binning).
    fn scale(&self, factor: f64) -> Img {
        let (w, h) = (
            (self.w as f64 * factor).round() as usize,
            (self.h as f64 * factor).round() as usize,
        );
        let ss = 4;
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for sy in 0..ss {
                    for sx in 0..ss {
                        let fx = (x as f64 + (sx as f64 + 0.5) / ss as f64) / factor - 0.5;
                        let fy = (y as f64 + (sy as f64 + 0.5) / ss as f64) / factor - 0.5;
                        acc += self.bilinear(fx, fy, 255.0);
                    }
                }
                px.push(acc / (ss * ss) as f32);
            }
        }
        Img { w, h, px }
    }

    fn blur(&self, sigma: f64) -> Img {
        if sigma <= 0.0 {
            return self.clone();
        }
        let r = (sigma * 3.0).ceil() as isize;
        let k: Vec<f32> = (-r..=r)
            .map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp() as f32)
            .collect();
        let s: f32 = k.iter().sum();
        let k: Vec<f32> = k.iter().map(|v| v / s).collect();
        let mut tmp = self.clone();
        for y in 0..self.h {
            for x in 0..self.w {
                tmp.px[y * self.w + x] = (-r..=r)
                    .map(|i| k[(i + r) as usize] * self.at(x as isize + i, y as isize, 255.0))
                    .sum();
            }
        }
        let mut out = tmp.clone();
        for y in 0..self.h {
            for x in 0..self.w {
                out.px[y * self.w + x] = (-r..=r)
                    .map(|i| k[(i + r) as usize] * tmp.at(x as isize, y as isize + i, 255.0))
                    .sum();
            }
        }
        out
    }

    /// `margin` px of `bg` on every side (room to rotate without clipping the code).
    fn pad(&self, margin: usize, bg: f32) -> Img {
        let (w, h) = (self.w + 2 * margin, self.h + 2 * margin);
        let mut px = vec![bg; w * h];
        for y in 0..self.h {
            px[(y + margin) * w + margin..(y + margin) * w + margin + self.w]
                .copy_from_slice(&self.px[y * self.w..(y + 1) * self.w]);
        }
        Img { w, h, px }
    }

    /// Rotates about the centre inside a canvas of the same size, `bg` outside.
    fn rotate(&self, deg: f64, bg: f32) -> Img {
        let (s, c) = deg.to_radians().sin_cos();
        let (cx, cy) = (self.w as f64 / 2.0, self.h as f64 / 2.0);
        let mut out = self.clone();
        for y in 0..self.h {
            for x in 0..self.w {
                let (dx, dy) = (x as f64 - cx, y as f64 - cy);
                out.px[y * self.w + x] =
                    self.bilinear(cx + c * dx + s * dy, cy - s * dx + c * dy, bg);
            }
        }
        out
    }

    /// Pastes `self` centred on a `w` x `h` canvas of grey, with a lighting gradient and noise.
    fn on_table(&self, w: usize, h: usize, noise: f64, rng: &mut Rng) -> Img {
        let (ox, oy) = ((w - self.w) / 2, (h - self.h) / 2);
        let mut px = vec![110.0f32; w * h];
        for y in 0..self.h {
            for x in 0..self.w {
                px[(y + oy) * w + x + ox] = self.px[y * self.w + x];
            }
        }
        for y in 0..h {
            for x in 0..w {
                let light = 0.75 + 0.25 * (x as f32 / w as f32);
                px[y * w + x] = px[y * w + x] * light + (noise * rng.gauss()) as f32;
            }
        }
        Img { w, h, px }
    }

    fn bytes(&self) -> Vec<u8> {
        self.px
            .iter()
            .map(|v| v.round().clamp(0.0, 255.0) as u8)
            .collect()
    }
}

fn scan(img: &Img) -> Vec<Vec<u8>> {
    let b = img.bytes();
    decode_qr(Gray {
        width: img.w,
        height: img.h,
        pixels: &b,
    })
}

/// A PBM (P4) the page's canvas was dumped to: 0 = black.
fn pbm(name: &str) -> Img {
    let b = fixture(name);
    let mut it = b.splitn(4, |c| *c == b'\n');
    assert_eq!(it.next().unwrap(), b"P4");
    let dims = String::from_utf8(it.next().unwrap().to_vec()).unwrap();
    let (w, h) = dims.split_once(' ').unwrap();
    let (w, h): (usize, usize) = (w.parse().unwrap(), h.parse().unwrap());
    let data = it.next().unwrap();
    let row = w.div_ceil(8);
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let black = data[y * row + x / 8] & (0x80 >> (x % 8)) != 0;
            px.push(if black { 0.0 } else { 255.0 });
        }
    }
    Img { w, h, px }
}

fn page_frames() -> Vec<Img> {
    (0..3).map(|i| pbm(&format!("qr-frame-0{i}.pbm"))).collect()
}

/// A frame drawn the way `drawQr` does: black modules on white, 4 module quiet zone.
fn draw_qr(frame: &[u8], scale: usize) -> Img {
    let code = QrCode::with_error_correction_level(frame, EcLevel::M).unwrap();
    let n = code.width();
    let quiet = 4;
    let size = (n + 2 * quiet) * scale;
    let colors = code.to_colors();
    let mut px = vec![255.0; size * size];
    for y in 0..size {
        for x in 0..size {
            let (mx, my) = (x / scale, y / scale);
            if mx >= quiet
                && my >= quiet
                && mx < n + quiet
                && my < n + quiet
                && colors[(my - quiet) * n + mx - quiet] == Color::Dark
            {
                px[y * size + x] = 0.0;
            }
        }
    }
    Img {
        w: size,
        h: size,
        px,
    }
}

// ------------------------------------------------------------------ QR tests

fn push_all(c: &mut ReplyCollector, frames: &[&Vec<u8>]) -> Vec<Scan> {
    frames.iter().map(|f| c.push_frame(f)).collect()
}

#[test]
fn page_canvas_frames_decode_and_assemble_to_the_exact_reply() {
    let frames = page_frames();
    let want = fixture("response-full.bin");
    assert_eq!(want.len(), 711);
    let payloads: Vec<Vec<u8>> = frames
        .iter()
        .map(|f| {
            let mut found = scan(f);
            assert_eq!(found.len(), 1);
            found.remove(0)
        })
        .collect();
    let mut c = ReplyCollector::new();
    let out = push_all(&mut c, &payloads.iter().collect::<Vec<_>>());
    assert_eq!(out[0], Scan::Progress(1, 3));
    assert_eq!(out[1], Scan::Progress(2, 3));
    assert_eq!(out[2], Scan::Complete(want.clone()));
    assert!(ShareResponseWire::decode(&want).is_ok());
}

#[test]
fn page_frames_out_of_order_with_duplicates_and_laps() {
    let frames = page_frames();
    let want = fixture("response-full.bin");
    let p: Vec<Vec<u8>> = frames.iter().map(|f| scan(f).remove(0)).collect();
    // lap 1 lost frame 1, lap 2 arrives backwards, with repeats (a camera is faster than 8 fps)
    let order = [2usize, 2, 0, 0, 0, 2, 2, 1];
    let mut c = ReplyCollector::new();
    let mut done = None;
    for (n, i) in order.iter().enumerate() {
        if let Scan::Complete(d) = c.push_frame(&p[*i]) {
            done = Some((n, d));
        }
    }
    assert_eq!(done, Some((7, want)));
}

/// Loops of a bigger document: frames made by the terminal's own encoder (an 8 KB reply is far
/// above anything real) drawn through the `qrcode` crate, shuffled, thinned, repeated.
#[test]
fn synthetic_loop_missing_frames_until_the_next_lap() {
    let doc: Vec<u8> = (0..3000u32).map(|i| (i * 31 + 7) as u8).collect();
    let frames = fountain_encode(&doc, 250, 25).unwrap();
    assert!(frames.len() > 12);
    let imgs: Vec<Img> = frames.iter().map(|f| draw_qr(f, 5)).collect();
    let mut rng = Rng(12345);
    let mut c = ReplyCollector::new();
    let mut seen = 0usize;
    let mut done = None;
    'laps: for lap in 0..8 {
        // each lap: a random 60% of the frames, in random order, some twice
        let mut idx: Vec<usize> = (0..imgs.len()).filter(|_| rng.unit() < 0.6).collect();
        for i in (1..idx.len()).rev() {
            idx.swap(i, (rng.next() % (i as u64 + 1)) as usize);
        }
        let dup: Vec<usize> = idx.iter().copied().filter(|_| rng.unit() < 0.3).collect();
        idx.extend(dup);
        for i in idx {
            seen += 1;
            if let Scan::Complete(d) = c.push_image(Gray {
                width: imgs[i].w,
                height: imgs[i].h,
                pixels: &imgs[i].bytes(),
            }) {
                done = Some((lap, d));
                break 'laps;
            }
        }
    }
    let (lap, got) = done.expect("complete within 8 laps");
    println!(
        "synthetic 3000 B / {} frames: complete in lap {} after {seen} frames",
        frames.len(),
        lap + 1
    );
    assert_eq!(got, doc);
    assert!(lap >= 1, "60% laps cannot normally finish in the first lap");
}

#[test]
fn a_foreign_fountain_document_does_not_block_the_reply() {
    let other = fountain_encode(&[9u8; 600], 250, 25).unwrap();
    let doc = fountain_encode(&[3u8; 700], 250, 25).unwrap();
    let mut c = ReplyCollector::new();
    c.push_frame(&other[0]);
    // the second document keeps looping: after enough foreign frames the collector follows it
    let mut done = None;
    for _ in 0..10 {
        for f in &doc {
            if let Scan::Complete(d) = c.push_frame(f) {
                done = Some(d);
            }
        }
    }
    assert_eq!(done, Some(vec![3u8; 700]));
}

#[test]
fn noise_that_is_not_a_fountain_frame_is_ignored() {
    let mut c = ReplyCollector::new();
    assert_eq!(
        c.push_frame(b"WIFI:S:guest;T:WPA;P:secret;;"),
        Scan::Nothing
    );
    assert_eq!(c.push_frame(&[]), Scan::Nothing);
    let blank = vec![255u8; 640 * 480];
    assert_eq!(
        c.push_image(Gray {
            width: 640,
            height: 480,
            pixels: &blank
        }),
        Scan::Nothing
    );
}

/// Camera-like variants of the page's own frames: area-scaled to a module size, blurred, rotated,
/// noisy, on a 640x480 table. Prints the decode rate per row.
#[test]
fn qr_robustness_table() {
    let frames = page_frames();
    let source_px_per_module = 5.0; // the pbm is 5 px per module
    let mut rng = Rng(2024);
    let mut hard_fail = Vec::new();
    println!("variant                                     decoded/3");
    let mut row = |label: String, must: bool, mk: &dyn Fn(&Img, &mut Rng) -> Img| {
        let ok = frames
            .iter()
            .filter(|f| {
                let img = mk(f, &mut rng);
                scan(&img).len() == 1
            })
            .count();
        println!(
            "{label:42}  {ok}/3{}",
            if must { "  (required)" } else { "" }
        );
        if must && ok != 3 {
            hard_fail.push(label);
        }
    };
    for ppm in [2.0, 2.5, 3.0, 4.0, 5.0] {
        let factor = ppm / source_px_per_module;
        row(format!("scale {ppm} px/module"), ppm >= 3.0, &|f, r| {
            f.scale(factor).on_table(640, 480, 0.0, r)
        });
    }
    for sigma in [0.5, 1.0, 1.5, 2.0] {
        row(
            format!("4 px/module blur sigma {sigma}"),
            sigma <= 1.0,
            &|f, r| f.scale(0.8).blur(sigma).on_table(640, 480, 0.0, r),
        );
    }
    for noise in [4.0, 8.0, 16.0, 24.0] {
        row(
            format!("4 px/module sensor noise {noise}"),
            noise <= 8.0,
            &|f, r| f.scale(0.8).blur(0.7).on_table(640, 480, noise, r),
        );
    }
    // a camera rotates first and bins afterwards: rotate at the source's 5 px per module
    for deg in [3.0, 8.0, 15.0, 30.0, 45.0, 90.0] {
        row(
            format!("5 px/module rotated {deg} deg"),
            deg <= 15.0,
            &|f, r| {
                f.pad(60, 255.0)
                    .rotate(deg, 255.0)
                    .blur(0.7)
                    .on_table(640, 480, 2.0, r)
            },
        );
    }
    for deg in [3.0, 8.0, 15.0, 30.0, 45.0] {
        row(format!("4 px/module rotated {deg} deg"), false, &|f, r| {
            f.pad(60, 255.0)
                .rotate(deg, 255.0)
                .scale(0.8)
                .blur(0.7)
                .on_table(640, 480, 2.0, r)
        });
    }
    row(
        "4 px/module, rot 8, blur 1, noise 6".to_string(),
        false,
        &|f, r| {
            f.pad(60, 255.0)
                .rotate(8.0, 255.0)
                .scale(0.8)
                .blur(1.0)
                .on_table(640, 480, 6.0, r)
        },
    );
    row(
        "3 px/module, rot 5, blur 1, noise 4".to_string(),
        false,
        &|f, r| {
            f.pad(60, 255.0)
                .rotate(5.0, 255.0)
                .scale(0.6)
                .blur(1.0)
                .on_table(640, 480, 4.0, r)
        },
    );
    assert!(
        hard_fail.is_empty(),
        "required variants failed: {hard_fail:?}"
    );
}

#[test]
fn rgb_and_yuyv_conversions() {
    use super::qrscan::{gray_from_rgb, gray_from_yuyv};
    assert_eq!(gray_from_rgb(&[255, 255, 255, 0, 0, 0]), vec![255, 0]);
    assert_eq!(gray_from_yuyv(&[10, 128, 20, 128]), vec![10, 20]);
}

/// Writes the camera-like variants as PGM files for comparing decoders outside the crate:
/// `SSHUB_DUMP_DIR=/tmp/qr cargo test dump_qr_variants -- --ignored`.
#[test]
#[ignore]
fn dump_qr_variants() {
    let dir = std::env::var("SSHUB_DUMP_DIR").expect("SSHUB_DUMP_DIR");
    std::fs::create_dir_all(&dir).unwrap();
    let mut rng = Rng(77);
    for (fi, f) in page_frames().iter().enumerate() {
        for ppm in [3.0, 4.0, 5.0, 6.0] {
            for sigma in [0.7, 1.2, 1.7, 2.2] {
                for rot in [0.0, 10.0, 30.0] {
                    let img = f
                        .pad(f.w / 5, 255.0)
                        .rotate(rot, 255.0)
                        .scale(ppm / 5.0)
                        .blur(sigma)
                        .on_table(1280, 720, 4.0, &mut rng);
                    let mut out = format!("P5\n{} {}\n255\n", img.w, img.h).into_bytes();
                    out.extend(img.bytes());
                    std::fs::write(format!("{dir}/p{ppm}_s{sigma}_r{rot}_f{fi}.pgm"), out).unwrap();
                }
            }
        }
    }
    // rotation alone, at the page's own 5 px per module, with and without a little blur
    let rot_dir = format!("{dir}/rot");
    std::fs::create_dir_all(&rot_dir).unwrap();
    for (fi, f) in page_frames().iter().enumerate() {
        for deg in [0.0, 3.0, 6.0, 8.0, 10.0, 15.0, 20.0, 30.0, 45.0] {
            for sigma in [0.0, 0.7] {
                let img = f
                    .pad(f.w / 5, 255.0)
                    .rotate(deg, 255.0)
                    .blur(sigma)
                    .on_table(800, 600, 0.0, &mut rng);
                let mut out = format!("P5\n{} {}\n255\n", img.w, img.h).into_bytes();
                out.extend(img.bytes());
                std::fs::write(format!("{rot_dir}/rot{deg}_b{sigma}_f{fi}.pgm"), out).unwrap();
            }
        }
    }
}

// ------------------------------------------------------------------ API

#[test]
fn emitter_forwards_only_a_valid_share_response_wire() {
    let (tx, rx) = std::sync::mpsc::channel();
    let em = Emitter::new(MediaKind::Audio, tx, super::ListenOpts::SHARE_RESPONSE);
    assert!(!em.reply(b"not a wire".to_vec()));
    assert!(matches!(
        rx.try_recv(),
        Ok(MediaEvent::Status(MediaKind::Audio, _))
    ));
    assert!(rx.try_recv().is_err());
    let wire = short_reply();
    assert!(em.reply(wire.clone()));
    assert_eq!(rx.try_recv(), Ok(MediaEvent::Reply(MediaKind::Audio, wire)));
}

#[cfg(not(any(feature = "audio", feature = "camera")))]
#[test]
fn without_the_features_nothing_is_available_and_listen_says_why() {
    assert_eq!(super::available(), super::MediaSupport::default());
    assert!(super::devices().is_empty());
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = super::listen(&[MediaKind::Audio, MediaKind::Camera, MediaKind::Audio], tx);
    drop(handle);
    let events: Vec<MediaEvent> = rx.try_iter().collect();
    assert_eq!(events.len(), 2, "each kind once: {events:?}");
    assert!(events
        .iter()
        .all(|e| matches!(e, MediaEvent::Status(_, s) if s.contains("built without"))));
}

fn enroll_docs() -> Vec<Vec<u8>> {
    use crate::sync::wire::EnrollFrame;
    vec![
        EnrollFrame::Commit {
            role: 0,
            commit: [7; 32],
        }
        .encode(),
        // a reveal is big (ML-KEM key): several QR frames
        EnrollFrame::Reveal {
            role: 0,
            reveal: (0..1300u32).map(|i| (i * 13 + 5) as u8).collect(),
        }
        .encode(),
        EnrollFrame::Confirm {
            role: 0,
            ok: true,
            sig: [9; 64],
        }
        .encode(),
    ]
}

/// EnrollFrame documents as the phone shows them (looping fountain frames, drawn to images),
/// read through `qrscan`: each document completes and parses back to the same frame.
#[test]
fn enroll_frames_through_qrscan() {
    let mut c = ReplyCollector::new();
    for doc in enroll_docs() {
        let frames = fountain_encode(&doc, 250, 25).unwrap();
        let mut done = None;
        for lap in 0..2 {
            for f in &frames {
                let img = draw_qr(f, 5);
                let b = img.bytes();
                if let Scan::Complete(d) = c.push_image(Gray {
                    width: img.w,
                    height: img.h,
                    pixels: &b,
                }) {
                    done = Some((lap, d));
                }
            }
        }
        let (_, got) = done.unwrap_or_else(|| {
            panic!(
                "document of {} B ({} frames) completes",
                doc.len(),
                frames.len()
            )
        });
        assert_eq!(got, doc);
        assert!(super::accept_enroll_frame(&got));
        assert!(!super::accept_share_response(&got));
    }
}

#[test]
fn keep_listening_hands_over_each_new_document_once() {
    use super::ListenOpts;
    let (tx, rx) = std::sync::mpsc::channel();
    let em = Emitter::new(MediaKind::Camera, tx, ListenOpts::ENROLL_FRAMES);
    let docs = enroll_docs();
    assert!(
        !em.reply(b"junk".to_vec()),
        "a refused document does not stop the channel"
    );
    assert!(matches!(rx.try_recv(), Ok(MediaEvent::Status(..))));
    for d in [&docs[0], &docs[0], &docs[1], &docs[1], &docs[0], &docs[2]] {
        assert!(
            !em.reply(d.clone()),
            "keep_listening never stops the channel"
        );
    }
    let got: Vec<Vec<u8>> = rx
        .try_iter()
        .map(|e| match e {
            MediaEvent::Reply(MediaKind::Camera, b) => b,
            other => panic!("{other:?}"),
        })
        .collect();
    // the immediate repeat is dropped, a return to an earlier document is a new step
    assert_eq!(
        got,
        vec![
            docs[0].clone(),
            docs[1].clone(),
            docs[0].clone(),
            docs[2].clone()
        ]
    );
    // without keep_listening the first accepted reply ends the channel
    let (tx, rx) = std::sync::mpsc::channel();
    let em = Emitter::new(
        MediaKind::Camera,
        tx,
        ListenOpts {
            keep_listening: false,
            ..ListenOpts::ENROLL_FRAMES
        },
    );
    assert!(em.reply(docs[2].clone()));
    assert!(matches!(rx.try_recv(), Ok(MediaEvent::Reply(..))));
}

#[test]
fn frames_of_a_finished_document_do_not_shadow_the_next_one() {
    let docs = enroll_docs();
    let mut c = ReplyCollector::new();
    let first = fountain_encode(&docs[1], 250, 25).unwrap();
    let mut completed = 0;
    // the phone keeps looping the finished document for a while
    for f in first.iter().cycle().take(first.len() * 3) {
        if matches!(c.push_frame(f), Scan::Complete(_)) {
            completed += 1;
        }
    }
    assert_eq!(completed, 1);
    // the very first frame of the next document counts
    let next = fountain_encode(&docs[2], 250, 25).unwrap();
    assert_eq!(c.push_frame(&next[0]), Scan::Complete(docs[2].clone()));
}
