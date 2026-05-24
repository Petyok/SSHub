//! Translate `crossterm` events into byte sequences a remote xterm-compatible
//! shell expects.
//!
//! Covers printable characters, control + alt modifiers, arrows / Home / End /
//! PgUp / PgDn, function keys, Tab, Enter, Backspace, Esc, and mouse events
//! when the remote application has requested them.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// Encode a `KeyEvent` for transmission to the PTY child.
///
/// Returns `None` for events with no meaningful encoding (modifier-only
/// presses, key releases on platforms that report them, etc.).
pub fn encode(key: KeyEvent) -> Option<Vec<u8>> {
    let mods = key.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let shift = mods.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Char(ch) => Some(encode_char(ch, ctrl, alt, shift)),
        KeyCode::Enter => Some(b"\r".to_vec()),
        KeyCode::Tab => Some(b"\t".to_vec()),
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Backspace => Some(b"\x7f".to_vec()),
        KeyCode::Esc => Some(b"\x1b".to_vec()),
        KeyCode::Up => Some(arrow_seq('A', mods)),
        KeyCode::Down => Some(arrow_seq('B', mods)),
        KeyCode::Right => Some(arrow_seq('C', mods)),
        KeyCode::Left => Some(arrow_seq('D', mods)),
        KeyCode::Home => Some(arrow_seq('H', mods)),
        KeyCode::End => Some(arrow_seq('F', mods)),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        KeyCode::Insert => Some(b"\x1b[2~".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::F(n) => Some(fn_seq(n)),
        KeyCode::Null => Some(vec![0]),
        _ => None,
    }
}

fn encode_char(ch: char, ctrl: bool, alt: bool, _shift: bool) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4);
    if alt {
        buf.push(0x1b);
    }
    if ctrl {
        // xterm-style control: lowercase a-z → 1-26, '@' → 0, '[' → 27,
        // '\\' → 28, ']' → 29, '^' → 30, '_' → 31.
        let lower = ch.to_ascii_lowercase();
        let code: Option<u8> = match lower {
            'a'..='z' => Some((lower as u8) - b'a' + 1),
            '@' | ' ' => Some(0),
            '[' => Some(27),
            '\\' => Some(28),
            ']' => Some(29),
            '^' => Some(30),
            '_' | '?' => Some(31),
            _ => None,
        };
        if let Some(b) = code {
            buf.push(b);
            return buf;
        }
        // Fall through: ctrl + char with no mapping → just the char.
    }
    let mut tmp = [0u8; 4];
    let s = ch.encode_utf8(&mut tmp);
    buf.extend_from_slice(s.as_bytes());
    buf
}

/// Build a CSI sequence for arrow / Home / End keys with modifier-aware
/// parameters (xterm-style: CSI 1;<n> X where n = 1+shift+2*alt+4*ctrl).
fn arrow_seq(final_byte: char, mods: KeyModifiers) -> Vec<u8> {
    let mut param: u8 = 1;
    if mods.contains(KeyModifiers::SHIFT) {
        param += 1;
    }
    if mods.contains(KeyModifiers::ALT) {
        param += 2;
    }
    if mods.contains(KeyModifiers::CONTROL) {
        param += 4;
    }
    if param == 1 {
        format!("\x1b[{final_byte}").into_bytes()
    } else {
        format!("\x1b[1;{param}{final_byte}").into_bytes()
    }
}

// ── Mouse encoding ────────────────────────────────────────────

/// Encode a mouse event into the byte sequence expected by the remote app,
/// or `None` if either the remote hasn't enabled mouse reporting or the
/// event isn't relevant under the active protocol mode.
///
/// Origin convention: xterm uses 1-based row/col (top-left is `(1, 1)`).
/// `event_x` / `event_y` are 0-based columns/rows within the PTY body.
pub fn encode_mouse(
    event: MouseEvent,
    event_x: u16,
    event_y: u16,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    if mode == MouseProtocolMode::None {
        return None;
    }

    // Some events should be dropped depending on what the remote wants.
    let is_motion = matches!(event.kind, MouseEventKind::Drag(_) | MouseEventKind::Moved);
    if is_motion
        && matches!(
            mode,
            MouseProtocolMode::None | MouseProtocolMode::Press | MouseProtocolMode::PressRelease
        )
    {
        return None;
    }

    let is_release = matches!(event.kind, MouseEventKind::Up(_));
    if is_release && matches!(mode, MouseProtocolMode::Press) {
        return None;
    }

    // Determine button code per xterm spec.
    // Buttons 0/1/2 are left/middle/right. Scroll wheel is 64 (up) / 65 (down).
    // Drag adds 32 to the button code.
    let (button_code, _is_button_down) = match event.kind {
        MouseEventKind::Down(b) => (button_index(b), true),
        MouseEventKind::Up(_) => (3, false), // release
        MouseEventKind::Drag(b) => (button_index(b) + 32, true),
        MouseEventKind::ScrollUp => (64, true),
        MouseEventKind::ScrollDown => (65, true),
        MouseEventKind::Moved => return None,
        _ => return None,
    };

    let mut code = button_code;
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        code |= 4;
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        code |= 8;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        code |= 16;
    }

    let col = event_x.saturating_add(1) as u32;
    let row = event_y.saturating_add(1) as u32;

    match encoding {
        MouseProtocolEncoding::Sgr => {
            let release_marker = if matches!(event.kind, MouseEventKind::Up(_)) {
                'm'
            } else {
                'M'
            };
            // SGR uses the actual button code on release, signalled by `m`
            // instead of `M`. Recompute button_code for release events here.
            let sgr_button = match event.kind {
                MouseEventKind::Up(b) => {
                    let mut c = button_index(b);
                    if event.modifiers.contains(KeyModifiers::SHIFT) {
                        c |= 4;
                    }
                    if event.modifiers.contains(KeyModifiers::ALT) {
                        c |= 8;
                    }
                    if event.modifiers.contains(KeyModifiers::CONTROL) {
                        c |= 16;
                    }
                    c
                }
                _ => code,
            };
            Some(format!("\x1b[<{sgr_button};{col};{row}{release_marker}").into_bytes())
        }
        MouseProtocolEncoding::Utf8 | MouseProtocolEncoding::Default => {
            // Default encoding: CSI M Cb Cx Cy where each char is 32 + value.
            // Caps at 223 for the default encoding.
            let cb = (code as u32 + 32).min(255) as u8;
            let cx = (col + 32).min(255) as u8;
            let cy = (row + 32).min(255) as u8;
            Some(vec![0x1b, b'[', b'M', cb, cx, cy])
        }
    }
}

fn button_index(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

fn fn_seq(n: u8) -> Vec<u8> {
    match n {
        1 => b"\x1bOP".to_vec(),
        2 => b"\x1bOQ".to_vec(),
        3 => b"\x1bOR".to_vec(),
        4 => b"\x1bOS".to_vec(),
        5 => b"\x1b[15~".to_vec(),
        6 => b"\x1b[17~".to_vec(),
        7 => b"\x1b[18~".to_vec(),
        8 => b"\x1b[19~".to_vec(),
        9 => b"\x1b[20~".to_vec(),
        10 => b"\x1b[21~".to_vec(),
        11 => b"\x1b[23~".to_vec(),
        12 => b"\x1b[24~".to_vec(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }
    fn km(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_chars_pass_through() {
        assert_eq!(encode(k(KeyCode::Char('a'))).unwrap(), b"a");
        assert_eq!(encode(k(KeyCode::Char('Z'))).unwrap(), b"Z");
        assert_eq!(encode(k(KeyCode::Char('1'))).unwrap(), b"1");
    }

    #[test]
    fn enter_is_cr() {
        assert_eq!(encode(k(KeyCode::Enter)).unwrap(), b"\r");
    }

    #[test]
    fn backspace_is_del() {
        assert_eq!(encode(k(KeyCode::Backspace)).unwrap(), b"\x7f");
    }

    #[test]
    fn ctrl_c_is_etx() {
        assert_eq!(
            encode(km(KeyCode::Char('c'), KeyModifiers::CONTROL)).unwrap(),
            b"\x03"
        );
    }

    #[test]
    fn ctrl_a_is_soh() {
        assert_eq!(
            encode(km(KeyCode::Char('a'), KeyModifiers::CONTROL)).unwrap(),
            b"\x01"
        );
    }

    #[test]
    fn ctrl_d_is_eot() {
        assert_eq!(
            encode(km(KeyCode::Char('d'), KeyModifiers::CONTROL)).unwrap(),
            b"\x04"
        );
    }

    #[test]
    fn alt_a_is_esc_a() {
        assert_eq!(
            encode(km(KeyCode::Char('a'), KeyModifiers::ALT)).unwrap(),
            b"\x1ba"
        );
    }

    #[test]
    fn arrows_unmodified() {
        assert_eq!(encode(k(KeyCode::Up)).unwrap(), b"\x1b[A");
        assert_eq!(encode(k(KeyCode::Down)).unwrap(), b"\x1b[B");
        assert_eq!(encode(k(KeyCode::Right)).unwrap(), b"\x1b[C");
        assert_eq!(encode(k(KeyCode::Left)).unwrap(), b"\x1b[D");
    }

    #[test]
    fn ctrl_arrow_uses_modifier_param() {
        assert_eq!(
            encode(km(KeyCode::Up, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[1;5A"
        );
    }

    #[test]
    fn shift_tab_is_csi_z() {
        assert_eq!(encode(k(KeyCode::BackTab)).unwrap(), b"\x1b[Z");
    }

    #[test]
    fn pgup_pgdn() {
        assert_eq!(encode(k(KeyCode::PageUp)).unwrap(), b"\x1b[5~");
        assert_eq!(encode(k(KeyCode::PageDown)).unwrap(), b"\x1b[6~");
    }

    #[test]
    fn f1_f4_use_ss3() {
        assert_eq!(encode(k(KeyCode::F(1))).unwrap(), b"\x1bOP");
        assert_eq!(encode(k(KeyCode::F(4))).unwrap(), b"\x1bOS");
    }

    #[test]
    fn f5_plus_use_csi_tilde() {
        assert_eq!(encode(k(KeyCode::F(5))).unwrap(), b"\x1b[15~");
        assert_eq!(encode(k(KeyCode::F(12))).unwrap(), b"\x1b[24~");
    }

    #[test]
    fn esc() {
        assert_eq!(encode(k(KeyCode::Esc)).unwrap(), b"\x1b");
    }

    #[test]
    fn utf8_chars() {
        assert_eq!(
            encode(k(KeyCode::Char('é'))).unwrap(),
            "é".as_bytes().to_vec()
        );
    }

    // ── Mouse encoding tests ─────────────────────────────────

    fn mouse_event(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::empty(),
        }
    }

    #[test]
    fn mouse_none_mode_returns_none() {
        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 7);
        assert!(
            encode_mouse(m, 5, 7, MouseProtocolMode::None, MouseProtocolEncoding::Sgr,).is_none()
        );
    }

    #[test]
    fn sgr_left_press_encodes_correctly() {
        // Body row 7 in screen coords, button left, SGR mode.
        // Expected: ESC [ < 0 ; col+1 ; row+1 M
        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 4, 6);
        let bytes = encode_mouse(
            m,
            4,
            6,
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Sgr,
        )
        .unwrap();
        assert_eq!(bytes, b"\x1b[<0;5;7M");
    }

    #[test]
    fn sgr_left_release_uses_lowercase_m() {
        let m = mouse_event(MouseEventKind::Up(MouseButton::Left), 4, 6);
        let bytes = encode_mouse(
            m,
            4,
            6,
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Sgr,
        )
        .unwrap();
        assert_eq!(bytes, b"\x1b[<0;5;7m");
    }

    #[test]
    fn sgr_scroll_wheel_up_uses_button_64() {
        let m = mouse_event(MouseEventKind::ScrollUp, 9, 4);
        let bytes = encode_mouse(
            m,
            9,
            4,
            MouseProtocolMode::Press,
            MouseProtocolEncoding::Sgr,
        )
        .unwrap();
        assert_eq!(bytes, b"\x1b[<64;10;5M");
    }

    #[test]
    fn press_only_mode_drops_release() {
        let m = mouse_event(MouseEventKind::Up(MouseButton::Left), 1, 1);
        assert!(encode_mouse(
            m,
            1,
            1,
            MouseProtocolMode::Press,
            MouseProtocolEncoding::Sgr,
        )
        .is_none());
    }

    #[test]
    fn motion_dropped_when_remote_only_wants_buttons() {
        let m = mouse_event(MouseEventKind::Moved, 1, 1);
        assert!(encode_mouse(
            m,
            1,
            1,
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Sgr,
        )
        .is_none());
    }
}
