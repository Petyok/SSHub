//! The single place OSC 52 ("set clipboard") sequences are framed.
//!
//! Two callers need it: `app::util` copies a stored secret out of SSHub itself,
//! and the session relay forwards a clipboard write that an application inside
//! the PTY emitted. Both go through here so the framing — and the selector
//! decision below — exists exactly once.

use std::io::Write;

/// Frame an already-base64-encoded payload and write it to `out`.
///
/// The selector is pinned to `c` (host clipboard) instead of being forwarded.
/// That is a deliberate normalisation: a payload vt100 handed us with selector
/// `p` (the X11 *primary selection*) is relayed as `c`, i.e. it lands on the
/// host clipboard rather than a primary selection of its own. Terminals differ
/// wildly in which selectors they accept, and several drop the whole sequence
/// when they meet one they don't know — a copy that silently does nothing is
/// worse than a copy that lands one selection over.
///
/// `payload` is expected to be base64 already. For the relay path vt100 has
/// validated the alphabet before handing it over, which is what rules out
/// ESC/BEL injection; this helper deliberately adds no second validation.
pub(crate) fn write_b64_to(out: &mut impl Write, payload: &str) -> std::io::Result<()> {
    out.write_all(format!("\x1b]52;c;{payload}\x07").as_bytes())?;
    out.flush()
}

/// Base64-encode plain `text` and write it through [`write_b64_to`].
pub(crate) fn write_text_to(out: &mut impl Write, text: &str) -> std::io::Result<()> {
    write_b64_to(out, &base64_encode(text.as_bytes()))
}

/// [`write_b64_to`] against the process's stdout — the terminal hosting SSHub.
pub(crate) fn write_b64(payload: &str) -> std::io::Result<()> {
    write_b64_to(&mut std::io::stdout().lock(), payload)
}

/// [`write_text_to`] against the process's stdout.
pub(crate) fn write_text(text: &str) -> std::io::Result<()> {
    write_text_to(&mut std::io::stdout().lock(), text)
}

/// Tiny base64 (standard alphabet, padded). Inlined so we don't pull in
/// another crate for a single ~20 line helper.
pub(crate) fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut chunks = input.chunks_exact(3);
    for chunk in chunks.by_ref() {
        let b = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((b >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(b & 0x3f) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        1 => {
            let b = (rem[0] as u32) << 16;
            out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
            out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_payload_is_framed_with_the_c_selector() {
        let mut out = Vec::new();
        write_b64_to(&mut out, "R0VIRUlN").unwrap();
        assert_eq!(out, b"\x1b]52;c;R0VIRUlN\x07");
    }

    #[test]
    fn plain_text_is_encoded_then_framed() {
        let mut out = Vec::new();
        write_text_to(&mut out, "GEHEIM").unwrap();
        assert_eq!(out, b"\x1b]52;c;R0VIRUlN\x07");
    }

    #[test]
    fn both_entry_points_share_one_framing_path() {
        // Guards the review point that OSC 52 must have a single production
        // implementation: encoding "GEHEIM" and relaying its base64 must
        // produce byte-identical output.
        let mut from_text = Vec::new();
        write_text_to(&mut from_text, "GEHEIM").unwrap();
        let mut from_b64 = Vec::new();
        write_b64_to(&mut from_b64, &base64_encode(b"GEHEIM")).unwrap();
        assert_eq!(from_text, from_b64);
    }

    #[test]
    fn base64_encode_pads() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"GEHEIM"), "R0VIRUlN");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b"a"), "YQ==");
    }
}
