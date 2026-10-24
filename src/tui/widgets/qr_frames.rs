//! Looping QR codes for payloads that do not fit one code (the unlock modal's
//! phone path). Each frame is encoded at error correction M and drawn as
//! upper-half-block cells: the top module in the foreground, the bottom one in
//! the background, explicit black on white RGB with a 4-module quiet zone.
//!
//! The colours ignore the theme on purpose: phones do not reliably read
//! inverted or tinted codes, and the quiet zone must be light whatever the
//! terminal background is.

use std::time::{Duration, Instant};

use qrcode::{Color as Module, EcLevel, QrCode, Version};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// Frames shown per second.
pub const FPS: u32 = crate::sync::limits::QR_FPS;
/// Time one frame stays on screen.
pub const FRAME_PERIOD: Duration = Duration::from_millis(1000 / FPS as u64);
/// Quiet zone around the code, in modules.
const QUIET: usize = 4;
const BLACK: Color = Color::Rgb(0, 0, 0);
const WHITE: Color = Color::Rgb(255, 255, 255);

/// Modules per side of a normal QR code of `version` (1..=40).
pub fn modules(version: i16) -> usize {
    17 + 4 * version as usize
}

/// Terminal cells (columns, rows) a code of `version` needs, quiet zone
/// included. Two modules share a row, so height binds on a typical terminal.
pub fn cells_needed(version: i16) -> (u16, u16) {
    let side = modules(version) + 2 * QUIET;
    (side as u16, side.div_ceil(2) as u16)
}

/// The largest version that fits `w` columns by `h` rows (one module per
/// column, two per row, 4-module quiet zone); the rule lives in
/// `fountain::qr_version_for_area`, so the frames and the widget agree.
fn version_for_area(w: u16, h: u16) -> Option<i16> {
    crate::sync::fountain::qr_version_for_area(w, h).map(|v| v as i16)
}

/// What [`render`] decided for an area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrPlan {
    /// No frames to show.
    NoFrames,
    /// A frame is larger than the biggest QR version.
    TooLarge,
    /// The area cannot hold the version the frames need; this is the size that
    /// can.
    TooSmall { need_w: u16, need_h: u16 },
    /// Frames are drawn at this version.
    Fits { version: i16 },
}

/// The smallest version that holds every frame at ECC M. `NoFrames` for an
/// empty list, `TooLarge` when a frame exceeds version 40.
pub fn needed_version(frames: &[Vec<u8>]) -> Result<i16, QrPlan> {
    if frames.is_empty() {
        return Err(QrPlan::NoFrames);
    }
    let mut needed = 1i16;
    for f in frames {
        match QrCode::with_error_correction_level(f, EcLevel::M) {
            Ok(code) => {
                if let Version::Normal(v) = code.version() {
                    needed = needed.max(v);
                }
            }
            Err(_) => return Err(QrPlan::TooLarge),
        }
    }
    Ok(needed)
}

/// Decide how `frames` can be drawn in a `w` x `h` area: all frames share the
/// largest version that fits, so the code does not change size between
/// frames, provided every frame fits that version at ECC M.
pub fn plan(frames: &[Vec<u8>], w: u16, h: u16) -> QrPlan {
    let needed = match needed_version(frames) {
        Ok(v) => v,
        Err(plan) => return plan,
    };
    match version_for_area(w, h) {
        Some(version) if version >= needed => QrPlan::Fits { version },
        _ => {
            let (need_w, need_h) = cells_needed(needed);
            QrPlan::TooSmall { need_w, need_h }
        }
    }
}

/// Draw frame `index % frames.len()` centred in `area`. Returns the plan: when
/// it is not `Fits`, nothing is drawn and the caller says why.
pub fn render(buf: &mut Buffer, area: Rect, frames: &[Vec<u8>], index: usize) -> QrPlan {
    let plan = plan(frames, area.width, area.height);
    let QrPlan::Fits { version } = plan else {
        return plan;
    };
    let frame = &frames[index % frames.len()];
    let Ok(code) = QrCode::with_version(frame, Version::Normal(version), EcLevel::M) else {
        return QrPlan::TooLarge;
    };
    let colors = code.to_colors();
    let n = code.width();
    let side = n + 2 * QUIET;
    let (cw, ch) = cells_needed(version);
    let x0 = area.x + (area.width - cw) / 2;
    let y0 = area.y + (area.height - ch) / 2;
    // Module at grid (gx, gy) of the code plus its quiet zone; outside is light.
    let dark = |gx: usize, gy: usize| -> bool {
        gx >= QUIET
            && gy >= QUIET
            && gx < QUIET + n
            && gy < QUIET + n
            && colors[(gy - QUIET) * n + (gx - QUIET)] == Module::Dark
    };
    for row in 0..ch as usize {
        for col in 0..side {
            let top = dark(col, 2 * row);
            let bottom = 2 * row + 1 < side && dark(col, 2 * row + 1);
            if let Some(cell) = buf.cell_mut((x0 + col as u16, y0 + row as u16)) {
                cell.set_symbol("\u{2580}");
                cell.set_fg(if top { BLACK } else { WHITE });
                cell.set_bg(if bottom { BLACK } else { WHITE });
            }
        }
    }
    plan
}

/// Advance a looping frame counter from the app tick: one step per
/// [`FRAME_PERIOD`] elapsed since `last`, wrapping at `count`. No thread, no
/// timer of its own; the tick supplies `now`.
pub fn advance(frame: &mut usize, last: &mut Option<Instant>, now: Instant, count: usize) {
    if count == 0 {
        *frame = 0;
        return;
    }
    let Some(prev) = *last else {
        *last = Some(now);
        return;
    };
    let steps = (now.saturating_duration_since(prev).as_millis() / FRAME_PERIOD.as_millis()) as u32;
    if steps > 0 {
        *frame = (*frame + steps as usize) % count;
        *last = Some(prev + FRAME_PERIOD * steps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31) ^ seed)
            .collect()
    }

    /// Real fountain frames, drawn and read back from the cells, reassemble
    /// the original payload through the decoder.
    #[test]
    fn rendered_fountain_frames_reassemble_the_payload() {
        use crate::sync::fountain::{fountain_encode, frame_bytes_for_area, FountainDecoder};
        let area = Rect::new(2, 1, 90, 40);
        let secret = payload(700, 9);
        let fb = frame_bytes_for_area(area.width, area.height).unwrap();
        let frames = fountain_encode(&secret, fb, 25).unwrap();
        assert!(frames.len() > 3, "a payload that needs several frames");
        let mut decoder = FountainDecoder::new();
        for index in 0..frames.len() {
            let mut buf = Buffer::empty(Rect::new(0, 0, 100, 50));
            let QrPlan::Fits { version } = render(&mut buf, area, &frames, index) else {
                panic!("fountain frames must fit the area they were sized for");
            };
            let grid = read_back(&buf, area, version);
            // Which frame do these cells encode? No QR decoder is available,
            // so match against the encoding of every frame; exactly one fits.
            let n = modules(version);
            let matches: Vec<usize> = (0..frames.len())
                .filter(|&j| {
                    let code =
                        QrCode::with_version(&frames[j], Version::Normal(version), EcLevel::M)
                            .unwrap();
                    let colors = code.to_colors();
                    (0..grid.len()).all(|y| {
                        (0..grid.len()).all(|x| {
                            let inside =
                                (QUIET..QUIET + n).contains(&x) && (QUIET..QUIET + n).contains(&y);
                            grid[y][x]
                                == (inside && colors[(y - QUIET) * n + (x - QUIET)] == Module::Dark)
                        })
                    })
                })
                .collect();
            assert_eq!(matches, vec![index], "the cells show frame {index}");
            decoder.push(&frames[matches[0]]).unwrap();
        }
        assert_eq!(decoder.finish().unwrap(), Some(secret));
    }

    /// Read the drawn code back into a module matrix (quiet zone included),
    /// checking every cell is the exact half block in black or white.
    #[allow(clippy::needless_range_loop)] // `col` also addresses the buffer
    fn read_back(buf: &Buffer, area: Rect, version: i16) -> Vec<Vec<bool>> {
        let (cw, ch) = cells_needed(version);
        let side = modules(version) + 2 * QUIET;
        let x0 = area.x + (area.width - cw) / 2;
        let y0 = area.y + (area.height - ch) / 2;
        let mut grid = vec![vec![false; side]; side + 1];
        for row in 0..ch as usize {
            for col in 0..side {
                let c = buf.cell((x0 + col as u16, y0 + row as u16)).unwrap();
                assert_eq!(c.symbol(), "\u{2580}", "cell ({col},{row})");
                for (color, y) in [(c.fg, 2 * row), (c.bg, 2 * row + 1)] {
                    assert!(
                        color == BLACK || color == WHITE,
                        "{color:?} at ({col},{row})"
                    );
                    grid[y][col] = color == BLACK;
                }
            }
        }
        grid.truncate(side);
        grid
    }

    #[test]
    #[allow(clippy::needless_range_loop)] // `i` walks the matrix edge on both axes
    fn drawn_cells_decode_to_the_encoded_frame() {
        let frames = vec![payload(120, 1), payload(120, 2), payload(90, 3)];
        let area = Rect::new(3, 2, 90, 40);
        for index in 0..frames.len() {
            let mut buf = Buffer::empty(Rect::new(0, 0, 100, 50));
            let QrPlan::Fits { version } = render(&mut buf, area, &frames, index) else {
                panic!("must fit");
            };
            let grid = read_back(&buf, area, version);
            let want =
                QrCode::with_version(&frames[index], Version::Normal(version), EcLevel::M).unwrap();
            let n = want.width();
            assert_eq!(grid.len(), n + 2 * QUIET);
            for (y, row) in grid.iter().enumerate() {
                for (x, &dark) in row.iter().enumerate() {
                    let inside = x >= QUIET && y >= QUIET && x < QUIET + n && y < QUIET + n;
                    let expect =
                        inside && want.to_colors()[(y - QUIET) * n + (x - QUIET)] == Module::Dark;
                    assert_eq!(dark, expect, "module ({x},{y}) of frame {index}");
                }
            }
            // The quiet zone is light on all four sides.
            for i in 0..grid.len() {
                for q in 0..QUIET {
                    assert!(!grid[q][i] && !grid[grid.len() - 1 - q][i]);
                    assert!(!grid[i][q] && !grid[i][grid.len() - 1 - q]);
                }
            }
        }
    }

    #[test]
    fn the_area_version_matches_the_rule_the_fountain_module_documents() {
        // One module per column, two per row, 4-module quiet zone: the full
        // 80x24 holds version 5, a 120x40 area version 13. Height binds.
        assert_eq!(version_for_area(80, 24), Some(5));
        assert_eq!(version_for_area(120, 40), Some(13));
        assert_eq!(cells_needed(5), (45, 23));
        assert_eq!(version_for_area(44, 100), Some(4));
        assert_eq!(version_for_area(10, 10), None);
    }

    #[test]
    fn a_too_small_area_asks_for_the_size_the_frames_need() {
        // 300 bytes at ECC M needs version 12 or more; 80x24 holds only 5.
        let frames = vec![payload(300, 7)];
        let QrPlan::TooSmall { need_w, need_h } = plan(&frames, 80, 24) else {
            panic!("must not fit");
        };
        let needed = (1..=40)
            .find(|&v| QrCode::with_version(&frames[0], Version::Normal(v), EcLevel::M).is_ok())
            .unwrap();
        assert_eq!((need_w, need_h), cells_needed(needed));
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        let before = buf.clone();
        assert!(matches!(
            render(&mut buf, Rect::new(0, 0, 80, 24), &frames, 0),
            QrPlan::TooSmall { .. }
        ));
        assert_eq!(buf, before, "no unreadable code is drawn");
    }

    #[test]
    fn empty_and_oversized_payloads_are_reported() {
        assert_eq!(plan(&[], 200, 100), QrPlan::NoFrames);
        assert_eq!(plan(&[vec![0xAB; 4000]], 400, 200), QrPlan::TooLarge);
    }

    #[test]
    fn every_frame_uses_the_same_version() {
        let frames = vec![payload(20, 1), payload(150, 2)];
        let QrPlan::Fits { version } = plan(&frames, 90, 40) else {
            panic!("must fit");
        };
        assert_eq!(version, version_for_area(90, 40).unwrap());
    }

    #[test]
    fn frames_advance_at_eight_fps_and_wrap() {
        let t0 = Instant::now();
        let (mut frame, mut last) = (0usize, None);
        advance(&mut frame, &mut last, t0, 3);
        assert_eq!(frame, 0, "the first tick only starts the clock");
        advance(&mut frame, &mut last, t0 + Duration::from_millis(124), 3);
        assert_eq!(frame, 0);
        advance(&mut frame, &mut last, t0 + Duration::from_millis(126), 3);
        assert_eq!(frame, 1);
        // One second later: 8 more steps, wrapping over three frames.
        advance(&mut frame, &mut last, t0 + Duration::from_millis(1126), 3);
        assert_eq!(frame, (1 + 8) % 3);
    }
}
