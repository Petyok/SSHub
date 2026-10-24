//! Short authentication string for enrollment: four English BIP39 words (44 bits) from the
//! handshake transcript hash, read aloud on both screens. Words, not emoji: terminals do
//! not always render emoji and words can be dictated over the phone. The commitment
//! scheme around it (both sides commit to their ephemeral keys by hash before revealing)
//! is what stops a man in the middle from grinding keys until the string matches.

use bip39::Language;

use crate::sync::limits::SAS_WORDS;

/// Four words joined by hyphens, from the first 44 bits of `transcript_hash`
/// (11 bits per word, most significant bits first), e.g. `harbor-globe-lantern-mint`.
pub fn sas_words(transcript_hash: &[u8; 32]) -> String {
    let list = Language::English.word_list();
    let mut head = 0u64;
    for b in &transcript_hash[..6] {
        head = (head << 8) | u64::from(*b);
    }
    (0..SAS_WORDS)
        .map(|i| list[((head >> (48 - 11 * (i + 1))) & 0x7ff) as usize])
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extremes_and_shape() {
        assert_eq!(sas_words(&[0; 32]), "abandon-abandon-abandon-abandon");
        assert_eq!(sas_words(&[0xff; 32]), "zoo-zoo-zoo-zoo");
        let w = sas_words(&crate::sync::crypto::chain_hash(b"x"));
        assert_eq!(w.split('-').count(), 4);
        assert!(w.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
    }

    #[test]
    fn only_the_first_44_bits_matter() {
        let mut a = [7u8; 32];
        let w = sas_words(&a);
        a[31] ^= 0xff;
        a[6] ^= 0xff;
        assert_eq!(
            sas_words(&a),
            w,
            "bytes beyond the 44 bit prefix must not change the string"
        );
        a[0] ^= 0x80;
        assert_ne!(sas_words(&a), w);
        let mut b = [7u8; 32];
        b[5] ^= 0x01; // the low nibble of byte 5 (bits 44..48) is unused
        assert_eq!(sas_words(&b), w);
        b[5] ^= 0x01;
        b[5] ^= 0x10; // the high nibble is the last 4 bits of the fourth word
        assert_ne!(sas_words(&b), w);
    }
}
