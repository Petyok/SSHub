//! Short authentication string for enrollment: four English BIP39 words (44 bits) from the
//! handshake transcript hash, read aloud on both screens. Words, not emoji: terminals do
//! not always render emoji and words can be dictated over the phone. The commitment
//! scheme around it (both sides commit to their ephemeral keys by hash before revealing)
//! is what stops a man in the middle from grinding keys until the string matches.

use bip39::Language;

use super::DevicePublic;

use crate::limits::{HANDOVER_WORDS, SAS_WORDS};

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

/// The words the old phone and the screen of a terminal show for a phone handover (design 5, contract 4j):
/// the first 88 bits of BLAKE3 over `"sshub/handover/v1" || ed25519 || x25519 || mlkem || name_len u8 || name ||
/// old_phone_id(8)`, 11 bits per word, most significant bits first. `name` is at most 255 bytes.
pub fn handover_words(new_phone: &DevicePublic, name: &str, old_phone_id: &[u8; 8]) -> Vec<String> {
    let mut h = blake3::Hasher::new();
    h.update(b"sshub/handover/v1");
    h.update(&new_phone.ed25519_pub);
    h.update(&new_phone.x25519_pub);
    h.update(&new_phone.mlkem_pub);
    h.update(&[u8::try_from(name.len()).expect("a handover name is at most 255 bytes")]);
    h.update(name.as_bytes());
    h.update(old_phone_id);
    let digest = h.finalize();
    let list = Language::English.word_list();
    let mut head = 0u128;
    for b in &digest.as_bytes()[..11] {
        head = (head << 8) | u128::from(*b);
    }
    (0..HANDOVER_WORDS)
        .map(|i| list[((head >> (88 - 11 * (i + 1))) & 0x7ff) as usize].to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone() -> DevicePublic {
        DevicePublic {
            ed25519_pub: [1; 32],
            x25519_pub: [2; 32],
            mlkem_pub: vec![3; 5],
        }
    }

    #[test]
    fn handover_words_are_eight_and_bind_every_input() {
        let w = handover_words(&phone(), "new phone", &[9; 8]);
        assert_eq!(w.len(), HANDOVER_WORDS);
        assert_eq!(w, handover_words(&phone(), "new phone", &[9; 8]));
        let mut p = phone();
        p.mlkem_pub[0] ^= 1;
        assert_ne!(handover_words(&p, "new phone", &[9; 8]), w);
        assert_ne!(handover_words(&phone(), "new phonf", &[9; 8]), w);
        assert_ne!(handover_words(&phone(), "new phone", &[8; 8]), w);
        // the committed vectors (tests/vectors/crypto) pin the words for the page and the Android core
    }

    #[test]
    fn extremes_and_shape() {
        assert_eq!(sas_words(&[0; 32]), "abandon-abandon-abandon-abandon");
        assert_eq!(sas_words(&[0xff; 32]), "zoo-zoo-zoo-zoo");
        let w = sas_words(&crate::crypto::chain_hash(b"x"));
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
