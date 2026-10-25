//! The phone page this build was made with, identified by its release fingerprint.
//!
//! `assets/companion/FINGERPRINT` is written by the page build: the sha256 of
//! `sshub-companion.html` with the fingerprint slot zeroed. The page shows the same value on its
//! About screen; the terminal shows this one so the two can be compared by eye before the
//! phone is trusted with an enrollment.

/// The fingerprint of the companion page that ships with this build (64 hex digits).
pub const COMPANION_FINGERPRINT: &str = trimmed(include_str!("../../assets/companion/FINGERPRINT"));

/// The line shown next to the fingerprint wherever it appears.
pub const COMPARE_HINT: &str = "compare with the phone's About screen";

const fn trimmed(s: &str) -> &str {
    let b = s.as_bytes();
    let mut n = b.len();
    while n > 0 && matches!(b[n - 1], b'\n' | b'\r' | b' ') {
        n -= 1;
    }
    let (head, _) = b.split_at(n);
    match core::str::from_utf8(head) {
        Ok(s) => s,
        Err(_) => panic!("FINGERPRINT is not utf-8"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn the_fingerprint_is_the_sha256_of_the_page_with_its_slot_zeroed() {
        let page = include_str!("../../assets/companion/sshub-companion.html");
        assert_eq!(COMPANION_FINGERPRINT.len(), 64, "{COMPANION_FINGERPRINT:?}");
        assert!(COMPANION_FINGERPRINT.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(
            page.contains(COMPANION_FINGERPRINT),
            "the page carries its own fingerprint"
        );
        let zeroed = page.replace(COMPANION_FINGERPRINT, &"0".repeat(64));
        let digest: String = Sha256::digest(zeroed.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            digest, COMPANION_FINGERPRINT,
            "rebuild the page: node build.mjs"
        );
    }
}
