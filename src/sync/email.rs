//! The emailed code as key material (optional third factor). The code is at least 20
//! Crockford characters (100 bits) and is a KDF input for decrypting a fragment, never a
//! password compared against a stored value: without the bytes from the mail the fragment
//! cannot be opened. Rules around who may send it live in the design ("The third factor").
//!
//! Blob: `"SSHE" || 1 || salt(16) || nonce(24) || ct`, header as associated data, key from
//! Argon2id with the email profile.

use anyhow::{anyhow, bail, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305,
};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use super::codes::normalize_code;
use super::crypto::{try_argon2_derive, Argon2Profile, ARGON2_EMAIL};

const MAGIC: &[u8; 4] = b"SSHE";
const VERSION: u8 = 1;
const HEADER: usize = 4 + 1 + 16 + 24;
/// 20 characters of five bits each.
pub const MIN_EMAIL_CODE_CHARS: usize = 20;

fn key_for(code: &str, salt: &[u8; 16], profile: Argon2Profile) -> Result<Zeroizing<[u8; 32]>> {
    let code = normalize_code(code)?;
    if code.len() < MIN_EMAIL_CODE_CHARS {
        bail!(
            "an emailed code has at least {MIN_EMAIL_CODE_CHARS} characters, got {}",
            code.len()
        );
    }
    Ok(Zeroizing::new(try_argon2_derive(&code, salt, profile)?))
}

pub fn email_fragment_seal(fragment: &[u8], code: &str) -> Result<Vec<u8>> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    seal_with(fragment, code, ARGON2_EMAIL, salt, nonce)
}

pub(crate) fn seal_with(
    fragment: &[u8],
    code: &str,
    profile: Argon2Profile,
    salt: [u8; 16],
    nonce: [u8; 24],
) -> Result<Vec<u8>> {
    let key = key_for(code, &salt, profile)?;
    let mut out = [MAGIC.as_slice(), &[VERSION], &salt, &nonce].concat();
    let ct = XChaCha20Poly1305::new((&*key).into())
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: fragment,
                aad: &out,
            },
        )
        .expect("XChaCha20-Poly1305 encrypt");
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn email_fragment_open(blob: &[u8], code: &str) -> Result<Vec<u8>> {
    open_with(blob, code, ARGON2_EMAIL)
}

pub(crate) fn open_with(blob: &[u8], code: &str, profile: Argon2Profile) -> Result<Vec<u8>> {
    if blob.len() < HEADER + 16 || &blob[..4] != MAGIC {
        bail!("not an email fragment");
    }
    if blob[4] != VERSION {
        bail!("unsupported email fragment version {}", blob[4]);
    }
    let salt: [u8; 16] = blob[5..21].try_into().unwrap();
    let nonce = &blob[21..45];
    let key = key_for(code, &salt, profile)?;
    XChaCha20Poly1305::new((&*key).into())
        .decrypt(
            nonce.into(),
            Payload {
                msg: &blob[HEADER..],
                aad: &blob[..HEADER],
            },
        )
        .map_err(|_| anyhow!("wrong code or damaged fragment"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::codes::gen_email_code;

    const CHEAP: Argon2Profile = Argon2Profile {
        mem_kib: 64,
        iters: 1,
    };

    #[test]
    fn round_trip_and_rejections() {
        let code = gen_email_code();
        let blob = seal_with(b"fragment bytes", &code, CHEAP, [1; 16], [2; 24]).unwrap();
        assert_eq!(open_with(&blob, &code, CHEAP).unwrap(), b"fragment bytes");
        assert_eq!(
            open_with(&blob, &code.to_lowercase().replace('-', ""), CHEAP).unwrap(),
            b"fragment bytes"
        );
        let other = gen_email_code();
        assert!(open_with(&blob, &other, CHEAP).is_err());
        assert!(
            open_with(&blob, "ABCDE-FGHJK", CHEAP).is_err(),
            "too short to be a real code"
        );
        let mut t = blob.clone();
        t[6] ^= 1;
        assert!(
            open_with(&t, &code, CHEAP).is_err(),
            "header is authenticated"
        );
        assert!(
            seal_with(b"x", "ABCDE", CHEAP, [0; 16], [0; 24]).is_err(),
            "a short code cannot seal"
        );
    }
}
