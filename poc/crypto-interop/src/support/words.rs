//! Word-phrase codecs (stage-0 Task 8 and Task 12): BIP39 24-word ↔ 32 bytes,
//! EFF-7776 6-word phrases ↔ 10 bytes.

use anyhow::anyhow;

// ---------------------------------------------------------------- BIP39

/// 32 bytes → 24 English BIP39 words with the embedded checksum.
pub fn bip39_encode(bytes: &[u8; 32]) -> String {
    bip39::Mnemonic::from_entropy(bytes)
        .expect("32-byte entropy is always valid BIP39 input")
        .to_string()
}

/// 24 English BIP39 words → 32 bytes. Whitespace is normalized, the checksum
/// is validated, and a wrong word is named in the error.
pub fn bip39_decode(phrase: &str) -> anyhow::Result<[u8; 32]> {
    let phrase = phrase.split_whitespace().collect::<Vec<_>>().join(" ");
    let mnemonic = bip39::Mnemonic::parse_normalized(&phrase).map_err(|e| match e {
        bip39::Error::UnknownWord(idx) => {
            let word = phrase.split_whitespace().nth(idx).unwrap_or("<missing>");
            anyhow!("invalid BIP39 phrase: unknown word {idx} ({word:?})")
        }
        other => anyhow!("invalid BIP39 phrase: {other}"),
    })?;
    let (entropy, len) = mnemonic.to_entropy_array();
    if len != 32 {
        anyhow::bail!("expected 24-word (32-byte) phrase, got {len} bytes");
    }
    Ok(entropy[..32].try_into().expect("len checked to be 32"))
}

// ---------------------------------------------------------------- EFF-7776

/// The EFF large wordlist (7776 words), one word per line.
pub const EFF_LARGE: &str = include_str!("../../wordlists/eff_large.txt");

const fn pow6(base: u128) -> u128 {
    let mut acc: u128 = 1;
    let mut i = 0;
    while i < 6 {
        acc *= base;
        i += 1;
    }
    acc
}


fn eff_words() -> &'static [&'static str] {
    use std::sync::OnceLock;
    static LIST: OnceLock<Vec<&'static str>> = OnceLock::new();
    LIST.get_or_init(|| {
        let words: Vec<&'static str> = EFF_LARGE.lines().map(|l| l.trim_end()).collect();
        assert_eq!(
            words.len(),
            7776,
            "EFF wordlist must have exactly 7776 entries"
        );
        words
    })
}

/// 10 bytes → 6 EFF words: u128 big-endian, words emitted least-significant
/// first, `v >= 7776^6` rejected.
pub fn eff_encode(entropy: &[u8; 10]) -> anyhow::Result<String> {
    let mut v: u128 = 0;
    for b in entropy {
        v = (v << 8) | *b as u128;
    }
    if v >= pow6(7776) {
        anyhow::bail!("entropy out of range: v >= 7776^6");
    }
    let mut out = Vec::with_capacity(6);
    for _ in 0..6 {
        out.push(eff_words()[(v % 7776) as usize]);
        v /= 7776;
    }
    Ok(out.join(" "))
}

/// Decode a 6-word EFF phrase (case-insensitive, whitespace-split) back to its
/// 10 entropy bytes, big-endian.
pub fn eff_decode(phrase: &str) -> anyhow::Result<[u8; 10]> {
    let words: Vec<&str> = phrase.split_whitespace().collect();
    if words.len() != 6 {
        anyhow::bail!("EFF phrase must be exactly 6 words, got {}", words.len());
    }
    let mut v: u128 = 0;
    // Words are least-significant-first, so accumulate from the END:
    // v = v*7776 + idx over the reversed word sequence (stage-0:1751-1754).
    for w in words.iter().rev() {
        let idx = eff_words()
            .iter()
            .position(|c| c.eq_ignore_ascii_case(w))
            .ok_or_else(|| anyhow!("unknown EFF word: {w:?}"))?;
        v = v * 7776 + idx as u128;
    }
    if v >= pow6(7776) {
        anyhow::bail!("EFF phrase out of range: v >= 7776^6");
    }
    let mut out = [0u8; 10];
    for b in out.iter_mut().rev() {
        *b = (v & 0xFF) as u8;
        v >>= 8;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stage-0:1050-1068 pinned Trezor BIP39 vectors.
    #[test]
    fn bip39_pinned_vectors() {
        let zeros = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
        let ff = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote";
        assert_eq!(bip39_encode(&[0u8; 32]), zeros);
        assert_eq!(bip39_encode(&[0xff; 32]), "zoo ".repeat(23).trim_end().to_string() + " vote");
        assert_eq!(bip39_decode(zeros).unwrap(), [0u8; 32]);
        assert_eq!(bip39_decode(&bip39_encode(&[0xff; 32])).unwrap(), [0xff; 32]);
    }

    /// Decode normalizes whitespace and names the bad word.
    #[test]
    fn bip39_decode_normalizes_and_rejects() {
        let phrase = bip39_encode(&[7u8; 32]);
        let sloppy = format!("  {}  ", phrase.replace(' ', "\t"));
        assert_eq!(bip39_decode(&sloppy).unwrap(), [7u8; 32]);
        let bogus: Vec<String> = phrase
            .split_whitespace()
            .enumerate()
            .map(|(i, w)| if i == 0 { "bogus".to_string() } else { w.to_string() })
            .collect();
        let err = bip39_decode(&bogus.join(" ")).unwrap_err().to_string();
        assert!(err.contains("unknown word"), "error: {err}");
        assert!(err.contains("bogus"), "error names the word: {err}");
        // 12-word phrase must be rejected as wrong length for the vault.
        let twelve = bip39::Mnemonic::from_entropy(&[1u8; 16]).unwrap().to_string();
        assert!(bip39_decode(&twelve).is_err());
    }

    /// EFF-7776: zero entropy → 6× first word; roundtrip; case-insensitive
    /// decode; out-of-range rejection.
    #[test]
    fn eff_roundtrip_and_edges() {
        let zero = [0u8; 10];
        let phrase = eff_encode(&zero).unwrap();
        assert_eq!(phrase, "abacus abacus abacus abacus abacus abacus");
        assert_eq!(eff_decode(&phrase).unwrap(), zero);

        let e = [0x00u8, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99];
        let p = eff_encode(&e).unwrap();
        assert_eq!(eff_decode(&p).unwrap(), e);
        assert_eq!(eff_decode(&p.to_uppercase()).unwrap(), e);

        // v = 2^80-1 >= 7776^6 → rejected
        assert!(eff_encode(&[0xff; 10]).is_err());
    }
}
