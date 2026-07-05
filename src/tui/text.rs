//! Char-safe string helpers for rendering (never slice on byte indices).

/// Truncate to at most `max` characters, appending `…` when cut.
pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('\u{2026}');
        out
    }
}

/// Truncate to `width` chars (with `…`) or pad with spaces up to `width`.
pub fn pad_ellipsize(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len > width {
        ellipsize(s, width)
    } else {
        let mut out = s.to_string();
        out.extend(std::iter::repeat(' ').take(width - len));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsize_is_char_safe() {
        assert_eq!(ellipsize("привет-мир", 6), "приве\u{2026}");
        assert_eq!(ellipsize("abc", 6), "abc");
        assert_eq!(ellipsize("тег1 · тег2 · тег3", 8), "тег1 · \u{2026}");
    }

    #[test]
    fn pad_ellipsize_pads_by_chars() {
        assert_eq!(pad_ellipsize("мир", 5), "мир  ");
        assert_eq!(pad_ellipsize("мирный", 4), "мир\u{2026}");
    }
}
