//! Human-typed codes: Crockford base32 (no I, L, O, U; I and L read as 1, O reads as 0),
//! used for the short-code snapshot (16 characters, 80 bits) and the emailed code.

use anyhow::{bail, Result};
use rand_core::{OsRng, RngCore};

use crate::limits::SNAPSHOT_CODE_CHARS;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Big-endian bit packing, five bits per character, no padding.
pub fn crockford_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in bytes {
        acc = (acc << 8) | u32::from(*b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Uppercases, drops spaces and hyphens, maps O to 0 and I and L to 1; rejects anything else
/// outside the alphabet.
pub fn normalize_code(input: &str) -> Result<String> {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        let c = c.to_ascii_uppercase();
        let c = match c {
            ' ' | '-' | '\t' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !ALPHABET.contains(&(c as u8)) {
            bail!("character {c:?} is not part of a code");
        }
        out.push(c);
    }
    Ok(out)
}

/// Groups of `group` characters separated by hyphens: `K7Q2-9FMX-C41T-8PWD`.
pub fn format_groups(code: &str, group: usize) -> String {
    code.as_bytes()
        .chunks(group)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("-")
}

/// 16 characters (80 bits) of true randomness, shown in groups of four.
pub fn gen_snapshot_code() -> String {
    let mut b = [0u8; 10];
    OsRng.fill_bytes(&mut b);
    let code = crockford_encode(&b);
    debug_assert_eq!(code.len(), SNAPSHOT_CODE_CHARS);
    format_groups(&code, 4)
}

/// 26 characters (128 bits) for the emailed code, shown in groups of five.
pub fn gen_email_code() -> String {
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    format_groups(&crockford_encode(&b), 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_known_values() {
        assert_eq!(crockford_encode(&[0; 10]), "0000000000000000");
        assert_eq!(crockford_encode(&[0xff; 10]), "ZZZZZZZZZZZZZZZZ");
        assert_eq!(crockford_encode(&[0x80, 0x00]), "G000");
        assert_eq!(crockford_encode(b"foobar"), "CSQPYRK1E8");
    }

    #[test]
    fn normalisation_is_forgiving_about_look_alikes_and_strict_about_the_rest() {
        assert_eq!(
            normalize_code("k7q2-9fmx c41t-8pwd").unwrap(),
            "K7Q29FMXC41T8PWD"
        );
        assert_eq!(normalize_code("OIl0").unwrap(), "0110");
        assert!(normalize_code("U").is_err());
        assert!(normalize_code("K7Q2-9FM!").is_err());
        assert!(normalize_code("é").is_err());
    }

    #[test]
    fn generated_codes_have_the_promised_shape_and_are_fresh() {
        let a = gen_snapshot_code();
        assert_eq!(a.len(), 19);
        assert_eq!(normalize_code(&a).unwrap().len(), SNAPSHOT_CODE_CHARS);
        assert_ne!(a, gen_snapshot_code());
        let e = gen_email_code();
        assert_eq!(normalize_code(&e).unwrap().len(), 26);
    }
}
