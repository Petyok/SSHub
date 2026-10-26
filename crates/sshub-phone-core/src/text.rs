//! Text that comes from another machine never reaches a screen raw: a name from a terminal must not be able to reorder
//! or hide the text around it. Port of `safeText`, `iso` and the base64 helpers of `assets/companion/src/ops.mjs`.

/// Every control, format, line and paragraph separator character (`\p{Cc} \p{Cf} \p{Zl} \p{Zp}`: bidi controls,
/// zero-width and word-joiner characters, soft hyphen, tag characters), every space separator except U+0020, plus the
/// invisible fillers (U+034F, U+115F, U+1160, U+17B4, U+17B5, U+180E, U+2800, U+3164, U+FFA0) and the variation
/// selectors (U+FE00..U+FE0F, U+E0100..U+E01EF).
fn unsafe_char(c: char) -> bool {
    let u = c as u32;
    matches!(
        u,
        // Cc
        0x00..=0x1F | 0x7F..=0x9F
        // Cf (Unicode 15.1)
        | 0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x890..=0x891 | 0x8E2 | 0x180E | 0x200B..=0x200F
        | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD
        | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F
        // Zl, Zp
        | 0x2028 | 0x2029
        // Zs except U+0020
        | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x202F | 0x205F | 0x3000
        // fillers and variation selectors
        | 0x34F | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x2800 | 0x3164 | 0xFFA0 | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF
    )
}

/// Escapes every unsafe character as `\uXXXX` (or `\u{X}` above the BMP); everything else is kept.
pub fn safe_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if unsafe_char(c) {
            let u = c as u32;
            if u > 0xFFFF {
                out.push_str(&format!("\\u{{{u:x}}}"));
            } else {
                out.push_str(&format!("\\u{u:04x}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `1970-01-01T00:00:00Z` style (no fraction), UTC, for a unix time in seconds; the number itself when out of range.
pub fn iso(t: i64) -> String {
    // civil-from-days (Howard Hinnant), valid for the whole i64 range we accept below
    if !(-62_135_596_800..=253_402_300_799).contains(&t) {
        return t.to_string();
    }
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3_600, rem % 3_600 / 60, rem % 60)
}

/// Base64 (standard alphabet, padded) of bytes: the text form of every frame.
pub fn to_base64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// Strict base64 of pasted text: surrounding whitespace is trimmed, anything else outside the alphabet is refused.
pub fn from_base64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_bidi_fillers_and_spaces_are_escaped_the_plain_space_stays() {
        assert_eq!(safe_text("plain host-01 ok"), "plain host-01 ok");
        assert_eq!(safe_text("a\u{202e}b"), "a\\u202eb");
        assert_eq!(safe_text("a\nb\u{7f}"), "a\\u000ab\\u007f");
        for c in ['\u{2800}', '\u{ffa0}', '\u{17b4}', '\u{17b5}', '\u{3000}', '\u{a0}', '\u{2009}', '\u{2003}', '\u{200b}', '\u{feff}', '\u{ad}', '\u{2028}', '\u{fe0f}'] {
            let s = format!("a{c}b");
            assert_ne!(safe_text(&s), s, "U+{:04x} passes unchanged", c as u32);
        }
        assert_eq!(safe_text("\u{e0100}"), "\\u{e0100}");
        assert_eq!(safe_text("\u{e0041}"), "\\u{e0041}", "tag characters are format characters");
        assert_eq!(safe_text("\u{e0020}"), "\\u{e0020}");
        assert_eq!(safe_text("naïve 名前"), "naïve 名前");
    }

    #[test]
    fn iso_matches_the_js_form() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_893_452_400), "2030-01-01T00:00:00Z");
        assert_eq!(iso(1_900_000_000), "2030-03-17T17:46:40Z");
        assert_eq!(iso(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso(-1), "1969-12-31T23:59:59Z");
        assert_eq!(iso(i64::MAX), i64::MAX.to_string());
    }

    #[test]
    fn base64_is_strict() {
        assert_eq!(from_base64(" aGk= \n").unwrap(), b"hi");
        assert!(from_base64("aGk").is_none());
        assert!(from_base64("a*k=").is_none());
        assert_eq!(to_base64(b"hi"), "aGk=");
    }
}
