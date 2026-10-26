//! Channels: the same frames (kinds 1 to 10) over a looping QR animation, a camera, a sound and Bluetooth LE. Port of
//! `assets/companion/src/channels.mjs` and `scan.mjs`. Rust never touches the camera, the screen or the speaker: it
//! turns bytes into module matrices and PCM, and gray camera frames into documents.

use qrcode::{Color, EcLevel, QrCode};
use sshub_sync_proto::fountain::fountain_encode;

use crate::error::{CoreError, Result};
use crate::views::QrFrame;

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
}
