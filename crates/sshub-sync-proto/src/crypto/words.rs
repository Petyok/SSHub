//! Word phrase codecs. There are two lists and they must not be confused: a
//! 24 word BIP39 phrase carries 32 bytes (paper share, key export), a 6 word
//! EFF-7776 phrase carries about 77 bits (Vault master phrase). Six BIP39 words
//! would be 66 bits, below what the design promises.

use anyhow::{anyhow, bail, Result};

/// 32 bytes to 24 English BIP39 words with the embedded checksum.
pub fn bip39_encode(bytes: &[u8; 32]) -> String {
    bip39::Mnemonic::from_entropy(bytes)
        .expect("32 bytes are valid BIP39 entropy")
        .to_string()
}

/// 24 English BIP39 words to 32 bytes. Whitespace and case are normalised, the
/// checksum is validated, and a wrong word is named in the error.
pub fn bip39_decode(phrase: &str) -> Result<[u8; 32]> {
    let phrase = normalize(phrase);
    let mnemonic = bip39::Mnemonic::parse_normalized(&phrase).map_err(|e| match e {
        bip39::Error::UnknownWord(idx) => {
            let word = phrase.split(' ').nth(idx).unwrap_or("<missing>");
            anyhow!("invalid BIP39 phrase: unknown word {} ({word:?})", idx + 1)
        }
        other => anyhow!("invalid BIP39 phrase: {other}"),
    })?;
    let (entropy, len) = mnemonic.to_entropy_array();
    if len != 32 {
        bail!("expected a 24 word (32 byte) phrase, got {len} bytes");
    }
    Ok(entropy[..32].try_into().expect("length checked"))
}

/// Lowercase, single-spaced form used for hashing and comparison.
pub fn normalize(phrase: &str) -> String {
    phrase
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

const EFF_LARGE: &str = include_str!("../../assets/wordlists/eff_large.txt");
const EFF_N: u128 = 7776;

fn eff_words() -> &'static [&'static str] {
    use std::sync::OnceLock;
    static LIST: OnceLock<Vec<&'static str>> = OnceLock::new();
    LIST.get_or_init(|| {
        let words: Vec<&'static str> = EFF_LARGE
            .lines()
            .map(|l| l.trim_end())
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(
            words.len(),
            7776,
            "EFF word list must hold exactly 7776 words"
        );
        words
    })
}

const fn eff_range() -> u128 {
    EFF_N * EFF_N * EFF_N * EFF_N * EFF_N * EFF_N
}

/// 10 bytes to 6 EFF words: the bytes are a big-endian integer, words are emitted
/// least significant first, and a value of 7776^6 or more is rejected.
pub fn eff_encode(entropy: &[u8; 10]) -> Result<String> {
    let mut v: u128 = 0;
    for b in entropy {
        v = (v << 8) | u128::from(*b);
    }
    if v >= eff_range() {
        bail!("entropy out of range: value is 7776^6 or more");
    }
    let mut out = Vec::with_capacity(6);
    for _ in 0..6 {
        out.push(eff_words()[(v % EFF_N) as usize]);
        v /= EFF_N;
    }
    Ok(out.join(" "))
}

/// 6 EFF words (case-insensitive, any whitespace) back to the 10 bytes.
pub fn eff_decode(phrase: &str) -> Result<[u8; 10]> {
    let phrase = normalize(phrase);
    let words: Vec<&str> = phrase.split(' ').filter(|w| !w.is_empty()).collect();
    if words.len() != 6 {
        bail!("an EFF phrase has exactly 6 words, got {}", words.len());
    }
    let mut v: u128 = 0;
    for w in words.iter().rev() {
        let idx = eff_words()
            .iter()
            .position(|c| c == w)
            .ok_or_else(|| anyhow!("unknown EFF word {w:?}"))?;
        v = v * EFF_N + idx as u128;
    }
    let mut out = [0u8; 10];
    let be = v.to_be_bytes();
    out.copy_from_slice(&be[6..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bip39_pinned_vectors() {
        let zeros = format!("{} art", "abandon ".repeat(23).trim_end());
        assert_eq!(bip39_encode(&[0; 32]), zeros);
        assert_eq!(bip39_decode(&zeros).unwrap(), [0; 32]);
        assert_eq!(
            bip39_encode(&[0xff; 32]),
            format!("{} vote", "zoo ".repeat(23).trim_end())
        );
    }

    #[test]
    fn bip39_decode_normalises_and_names_the_bad_word() {
        let p = bip39_encode(&[7; 32]);
        assert_eq!(
            bip39_decode(&format!("  {}  ", p.to_uppercase().replace(' ', "\t"))).unwrap(),
            [7; 32]
        );
        let third = p.split(' ').nth(2).unwrap().to_string();
        let bad = p.replacen(&third, "notaword", 1);
        let err = bip39_decode(&bad).unwrap_err().to_string();
        assert!(err.contains("word 3") && err.contains("notaword"), "{err}");
        assert!(bip39_decode("abandon abandon").is_err());
    }

    #[test]
    fn eff_list_shape_and_round_trip() {
        assert_eq!(eff_words().len(), 7776);
        assert_eq!((eff_words()[0], eff_words()[7775]), ("abacus", "zoom"));
        let mut max = [0u8; 10];
        max.copy_from_slice(&(eff_range() - 1).to_be_bytes()[6..]);
        for ent in [[0u8; 10], [1u8; 10], max] {
            assert_eq!(eff_decode(&eff_encode(&ent).unwrap()).unwrap(), ent);
        }
        assert!(eff_encode(&[0xff; 10]).is_err(), "out of range");
        let p = eff_encode(&[1; 10]).unwrap();
        assert_eq!(eff_decode(&p.to_uppercase()).unwrap(), [1; 10]);
        assert!(eff_decode(&format!("{p} abacus")).is_err(), "7 words");
        assert!(eff_decode("abacus abacus abacus abacus abacus nope").is_err());
    }
}
