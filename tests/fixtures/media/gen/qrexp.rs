use std::fs;
fn pgm(path: &std::path::Path) -> (usize, usize, Vec<u8>) {
    let b = fs::read(path).unwrap();
    let mut it = b.splitn(4, |c| *c == b'\n');
    it.next();
    let d = String::from_utf8(it.next().unwrap().to_vec()).unwrap();
    it.next();
    let (w, h) = d.split_once(' ').unwrap();
    (w.parse().unwrap(), h.parse().unwrap(), it.next().unwrap().to_vec())
}
fn up2(w: usize, h: usize, px: &[u8]) -> (usize, usize, Vec<u8>) {
    let (nw, nh) = (w * 2, h * 2);
    let mut out = vec![0u8; nw * nh];
    for y in 0..nh {
        for x in 0..nw {
            let fx = (x as f32 + 0.5) / 2.0 - 0.5;
            let fy = (y as f32 + 0.5) / 2.0 - 0.5;
            let (x0, y0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (ax, ay) = ((fx - x0 as f32).clamp(0.0, 1.0), (fy - y0 as f32).clamp(0.0, 1.0));
            let g = |xx: usize, yy: usize| px[yy * w + xx] as f32;
            let v = (g(x0, y0) * (1.0 - ax) + g(x1, y0) * ax) * (1.0 - ay) + (g(x0, y1) * (1.0 - ax) + g(x1, y1) * ax) * ay;
            out[y * nw + x] = v as u8;
        }
    }
    (nw, nh, out)
}
fn rq(w: usize, h: usize, px: &[u8]) -> usize {
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| px[y * w + x]);
    img.detect_grids().iter().filter(|g| { let mut o = Vec::new(); g.decode_to(&mut o).is_ok() && !o.is_empty() }).count()
}
fn qc(qd: &mut quircs::Quirc, w: usize, h: usize, px: &[u8]) -> usize {
    qd.identify(w, h, px).filter_map(|c| c.ok()).filter(|c| c.decode().is_ok()).count()
}
fn rx(w: usize, h: usize, px: &[u8]) -> usize {
    use rxing::{common::HybridBinarizer, qrcode::QRCodeReader, BinaryBitmap, DecodeHints, Luma8LuminanceSource, Reader};
    let mut bitmap = BinaryBitmap::new(HybridBinarizer::new(Luma8LuminanceSource::new(px.to_vec(), w as u32, h as u32).unwrap()));
    let hints = DecodeHints::default().with(rxing::DecodeHintValue::TryHarder(true));
    match QRCodeReader::default().decode_with_hints(&mut bitmap, &hints) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}
fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let mut names: Vec<_> = fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|e| e == "pgm").unwrap_or(false)).collect();
    names.sort();
    let mut qd = quircs::Quirc::default();
    for p in names {
        let (w, h, px) = pgm(&p);
        let (w2, h2, px2) = up2(w, h, &px);
        let mut out = String::new();
        let mut run = |label: &str, f: &mut dyn FnMut() -> usize| {
            let t = std::time::Instant::now();
            let n = f();
            out += &format!(" {label}={n} ms_{label}={}", t.elapsed().as_millis());
        };
        run("rqrr", &mut || rq(w, h, &px));
        run("rqrr2x", &mut || rq(w2, h2, &px2));
        run("quircs", &mut || qc(&mut qd, w, h, &px));
        run("quircs2x", &mut || qc(&mut qd, w2, h2, &px2));
        run("rxing", &mut || rx(w, h, &px));
        println!("RESULT {}{}", p.file_name().unwrap().to_string_lossy(), out);
    }
}
