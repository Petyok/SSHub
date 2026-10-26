//! Rendering what the phone sends: the frame, its text form and (when asked for and short enough) the looping QR.
//! Port of `opsOut` and the output half of `send` in `assets/companion/src/app.mjs`.

use sshub_sync_proto::wire::{PhoneOpsWire, SignedOpBytes};

use crate::channels::{qr_frames, QR_PARITY_PERCENT};
use crate::error::Result;
use crate::text::to_base64;
use crate::views::Outgoing;

/// Beyond this a QR animation is too long: the text only (`MAX_QR_DOC_BYTES` of the page).
pub const MAX_QR_DOC_BYTES: usize = 12_000;

/// An `Outgoing` for a whole frame. `kind` is read from the frame header (`"SWR1" || version u16 || kind`).
pub fn render(frame: Vec<u8>, with_qr: bool, note: &str) -> Result<Outgoing> {
    let kind = frame.get(6).copied().unwrap_or(0);
    let too_big = frame.len() > MAX_QR_DOC_BYTES;
    let qr = if with_qr && !too_big { qr_frames(&frame, QR_PARITY_PERCENT)? } else { Vec::new() };
    Ok(Outgoing {
        kind,
        text: to_base64(&frame),
        qr,
        qr_too_big: with_qr && too_big,
        audio_frame: None,
        manual: None,
        short_code: None,
        note: note.to_string(),
        frame,
    })
}

/// A kind 5 list of ops for a terminal (QR and text), as every screen that produces signed operations shows it.
pub fn ops_out(ops: &[SignedOpBytes], note: &str) -> Result<Outgoing> {
    render(PhoneOpsWire { ops: ops.to_vec() }.encode(), true, note)
}
