//! Short-code snapshots. A share cannot be pre-encrypted for a repeat unlock (the phone only
//! learns it at challenge time), so after a successful release the phone ships the peer one
//! blob `AEAD(share, Argon2id(code, salt))`:
//!
//! * the code is 16 Crockford characters (80 bits) that exist only on the phone, single use;
//! * one live snapshot per credential, enforced by the phone (the issuer), not by the peer;
//! * TTL 7 days, recorded inside the blob; the Argon2 profile too, so a weakened profile from
//!   a slow phone is visible ([`SnapshotBlob::is_weakened`]);
//! * a DEK reissue invalidates it.
//!
//! Wire form: `"SSHN" || 1 || mem u32 BE || iters u32 BE || issued_at i64 BE || salt(16) ||
//! nonce(24) || ct`; everything before `ct` is associated data. `sealed` is `nonce || ct`.

use anyhow::{anyhow, bail, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305,
};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use super::codes::normalize_code;
use super::crypto::{try_argon2_derive, Argon2Profile, Share, ARGON2_SNAPSHOT};
use super::limits::{
    SNAPSHOT_ARGON2_FLOOR, SNAPSHOT_CODE_CHARS, SNAPSHOT_TTL_SECS, TIMESTAMP_SKEW_GRACE_SECS,
};

const MAGIC: &[u8; 4] = b"SSHN";
const VERSION: u8 = 1;
const HEADER: usize = 4 + 1 + 4 + 4 + 8 + 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotBlob {
    pub salt: [u8; 16],
    pub profile: Argon2Profile,
    pub issued_at: i64,
    /// `nonce(24) || ciphertext+tag`.
    pub sealed: Vec<u8>,
}

impl SnapshotBlob {
    /// True when the phone had to use less memory or fewer passes than the published floor.
    pub fn is_weakened(&self) -> bool {
        self.profile.mem_kib < SNAPSHOT_ARGON2_FLOOR.mem_kib
            || self.profile.iters < SNAPSHOT_ARGON2_FLOOR.iters
    }

    fn header(&self) -> Vec<u8> {
        let mut h = Vec::with_capacity(HEADER);
        h.extend_from_slice(MAGIC);
        h.push(VERSION);
        h.extend_from_slice(&self.profile.mem_kib.to_be_bytes());
        h.extend_from_slice(&self.profile.iters.to_be_bytes());
        h.extend_from_slice(&self.issued_at.to_be_bytes());
        h.extend_from_slice(&self.salt);
        h
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        [self.header(), self.sealed.clone()].concat()
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() < HEADER + 24 + 16 || &b[..4] != MAGIC {
            bail!("not a snapshot blob");
        }
        if b[4] != VERSION {
            bail!("unsupported snapshot version {}", b[4]);
        }
        Ok(Self {
            profile: Argon2Profile {
                mem_kib: u32::from_be_bytes(b[5..9].try_into().unwrap()),
                iters: u32::from_be_bytes(b[9..13].try_into().unwrap()),
            },
            issued_at: i64::from_be_bytes(b[13..21].try_into().unwrap()),
            salt: b[21..37].try_into().unwrap(),
            sealed: b[37..].to_vec(),
        })
    }
}

fn key_for(code: &str, salt: &[u8; 16], profile: Argon2Profile) -> Result<Zeroizing<[u8; 32]>> {
    let code = normalize_code(code)?;
    if code.len() != SNAPSHOT_CODE_CHARS {
        bail!(
            "a snapshot code has {SNAPSHOT_CODE_CHARS} characters, got {}",
            code.len()
        );
    }
    Ok(Zeroizing::new(try_argon2_derive(&code, salt, profile)?))
}

/// Seals `share` under `code` with the published floor profile (the phone may pass a weaker
/// one through [`snapshot_seal_with_profile`] and the TUI flags it).
pub fn snapshot_seal(share: &Share, code: &str, now: i64) -> SnapshotBlob {
    snapshot_seal_with_profile(share, code, now, ARGON2_SNAPSHOT).expect("a well formed code")
}

pub fn snapshot_seal_with_profile(
    share: &Share,
    code: &str,
    now: i64,
    profile: Argon2Profile,
) -> Result<SnapshotBlob> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    seal_with(share, code, now, profile, salt, nonce)
}

pub(crate) fn seal_with(
    share: &Share,
    code: &str,
    now: i64,
    profile: Argon2Profile,
    salt: [u8; 16],
    nonce: [u8; 24],
) -> Result<SnapshotBlob> {
    let key = key_for(code, &salt, profile)?;
    let mut blob = SnapshotBlob {
        salt,
        profile,
        issued_at: now,
        sealed: Vec::new(),
    };
    let mut plain = Zeroizing::new(vec![share.index]);
    plain.extend_from_slice(&share.bytes);
    let ct = XChaCha20Poly1305::new((&*key).into())
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: &plain,
                aad: &blob.header(),
            },
        )
        .expect("XChaCha20-Poly1305 encrypt");
    blob.sealed = [nonce.as_slice(), ct.as_slice()].concat();
    Ok(blob)
}

/// Opens a snapshot: TTL and profile bounds are checked before any Argon2 work.
pub fn snapshot_open(blob: &SnapshotBlob, code: &str, now: i64) -> Result<Share> {
    if blob.issued_at > now + TIMESTAMP_SKEW_GRACE_SECS {
        bail!("snapshot is dated in the future");
    }
    if now - blob.issued_at > SNAPSHOT_TTL_SECS {
        bail!("snapshot expired");
    }
    if blob.sealed.len() < 24 + 16 {
        bail!("snapshot payload too short");
    }
    blob.profile.validate()?;
    let key = key_for(code, &blob.salt, blob.profile)?;
    let (nonce, ct) = blob.sealed.split_at(24);
    let plain = Zeroizing::new(
        XChaCha20Poly1305::new((&*key).into())
            .decrypt(
                nonce.into(),
                Payload {
                    msg: ct,
                    aad: &blob.header(),
                },
            )
            .map_err(|_| anyhow!("wrong code or damaged snapshot"))?,
    );
    if plain.len() != 33 || plain[0] == 0 {
        bail!("snapshot holds a malformed share");
    }
    Ok(Share {
        index: plain[0],
        bytes: plain[1..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::codes::gen_snapshot_code;

    const CHEAP: Argon2Profile = Argon2Profile {
        mem_kib: 64,
        iters: 1,
    };

    fn share() -> Share {
        Share {
            index: 3,
            bytes: vec![0x5a; 32],
        }
    }

    #[test]
    fn round_trip_with_forgiving_code_entry() {
        let code = gen_snapshot_code();
        let blob = snapshot_seal_with_profile(&share(), &code, 1_000, CHEAP).unwrap();
        assert_eq!(snapshot_open(&blob, &code, 2_000).unwrap(), share());
        assert_eq!(
            snapshot_open(&blob, &code.to_lowercase().replace('-', " "), 2_000).unwrap(),
            share()
        );
        assert_eq!(SnapshotBlob::from_bytes(&blob.to_bytes()).unwrap(), blob);
    }

    #[test]
    fn rejections_happen_before_and_after_argon2() {
        let code = "K7Q2-9FMX-C41T-8PWD";
        let blob = snapshot_seal_with_profile(&share(), code, 1_000, CHEAP).unwrap();
        assert!(
            snapshot_open(&blob, "K7Q2-9FMX-C41T-8PWE", 1_001).is_err(),
            "wrong code"
        );
        assert!(
            snapshot_open(&blob, "K7Q2-9FMX-C41T", 1_001).is_err(),
            "short code"
        );
        assert!(
            snapshot_open(&blob, code, 1_000 + SNAPSHOT_TTL_SECS + 1).is_err(),
            "expired"
        );
        assert!(
            snapshot_open(&blob, code, 1_000 + SNAPSHOT_TTL_SECS).is_ok(),
            "last second of the term"
        );
        assert!(
            snapshot_open(&blob, code, 1_000 - TIMESTAMP_SKEW_GRACE_SECS - 1).is_err(),
            "dated in the future"
        );
        let mut t = blob.clone();
        t.issued_at += 1; // the header is associated data
        assert!(snapshot_open(&t, code, 1_001).is_err());
        let mut t = blob.clone();
        t.profile.iters += 1;
        assert!(snapshot_open(&t, code, 1_001).is_err());
        let mut bomb = blob.clone();
        bomb.profile = Argon2Profile {
            mem_kib: u32::MAX,
            iters: 1,
        };
        assert!(snapshot_open(&bomb, code, 1_001).is_err(), "memory bomb");
        assert!(SnapshotBlob::from_bytes(&blob.to_bytes()[..40]).is_err());
    }

    #[test]
    fn a_weakened_profile_is_flagged() {
        let blob = snapshot_seal_with_profile(&share(), "K7Q2-9FMX-C41T-8PWD", 0, CHEAP).unwrap();
        assert!(blob.is_weakened());
        let floor = SnapshotBlob {
            profile: SNAPSHOT_ARGON2_FLOOR,
            ..blob
        };
        assert!(!floor.is_weakened());
    }
}
