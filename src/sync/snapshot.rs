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
//!
//! Version 2 (what the page issues now) names the credential inside the associated data:
//! `"SSHN" || 2 || mem || iters || issued_at || salt(16) || host_uid(16) || nonce(24) || ct`. A blob
//! can then be refused for any credential but its own, and the terminal keeps at most one live blob
//! per credential. Version 1 stays readable (vectors, older phones) and is unbound.

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
const VERSION_1: u8 = 1;
const VERSION_2: u8 = 2;
const HEADER: usize = 4 + 1 + 4 + 4 + 8 + 16;
const HEADER_V2: usize = HEADER + 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotBlob {
    pub salt: [u8; 16],
    pub profile: Argon2Profile,
    pub issued_at: i64,
    /// The credential this blob is for (version 2); `None` for a version 1 blob.
    pub host_uid: Option<[u8; 16]>,
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
        h.push(if self.host_uid.is_some() { VERSION_2 } else { VERSION_1 });
        h.extend_from_slice(&self.profile.mem_kib.to_be_bytes());
        h.extend_from_slice(&self.profile.iters.to_be_bytes());
        h.extend_from_slice(&self.issued_at.to_be_bytes());
        h.extend_from_slice(&self.salt);
        if let Some(host) = &self.host_uid {
            h.extend_from_slice(host);
        }
        h
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        [self.header(), self.sealed.clone()].concat()
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() < HEADER + 24 + 16 || &b[..4] != MAGIC {
            bail!("not a snapshot blob");
        }
        let (host_uid, body) = match b[4] {
            VERSION_1 => (None, HEADER),
            VERSION_2 => {
                if b.len() < HEADER_V2 + 24 + 16 {
                    bail!("not a snapshot blob");
                }
                (Some(b[HEADER..HEADER_V2].try_into().unwrap()), HEADER_V2)
            }
            v => bail!("unsupported snapshot version {v}"),
        };
        Ok(Self {
            host_uid,
            profile: Argon2Profile {
                mem_kib: u32::from_be_bytes(b[5..9].try_into().unwrap()),
                iters: u32::from_be_bytes(b[9..13].try_into().unwrap()),
            },
            issued_at: i64::from_be_bytes(b[13..21].try_into().unwrap()),
            salt: b[21..37].try_into().unwrap(),
            sealed: b[body..].to_vec(),
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
    seal_with(share, code, now, profile, salt, nonce, None)
}

/// A version 2 blob: bound to `host`, sealed with the published floor profile unless the phone passes a
/// weaker one.
pub fn snapshot_seal_for(
    host: &[u8; 16],
    share: &Share,
    code: &str,
    now: i64,
    profile: Argon2Profile,
) -> Result<SnapshotBlob> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    seal_with(share, code, now, profile, salt, nonce, Some(*host))
}

pub(crate) fn seal_with(
    share: &Share,
    code: &str,
    now: i64,
    profile: Argon2Profile,
    salt: [u8; 16],
    nonce: [u8; 24],
    host_uid: Option<[u8; 16]>,
) -> Result<SnapshotBlob> {
    let key = key_for(code, &salt, profile)?;
    let mut blob = SnapshotBlob {
        salt,
        profile,
        issued_at: now,
        host_uid,
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

/// [`snapshot_open`] for the credential `host`: a version 2 blob for another credential is refused before
/// any Argon2 work. A version 1 blob names no credential and passes (the AEAD of the credential itself
/// is the later check).
pub fn snapshot_open_for(
    blob: &SnapshotBlob,
    host: &[u8; 16],
    code: &str,
    now: i64,
) -> Result<Share> {
    if blob.host_uid.is_some_and(|h| h != *host) {
        bail!("this snapshot is for another credential");
    }
    snapshot_open(blob, code, now)
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
    fn version_two_names_its_credential_and_binds_it_into_the_aead() {
        let code = "K7Q2-9FMX-C41T-8PWD";
        let (host, other) = ([7u8; 16], [8u8; 16]);
        let blob = snapshot_seal_for(&host, &share(), code, 1_000, CHEAP).unwrap();
        assert_eq!(blob.host_uid, Some(host));
        let wire = blob.to_bytes();
        assert_eq!(wire[4], 2);
        assert_eq!(&wire[37..53], &host);
        assert_eq!(SnapshotBlob::from_bytes(&wire).unwrap(), blob);
        assert_eq!(snapshot_open_for(&blob, &host, code, 1_001).unwrap(), share());
        assert!(snapshot_open_for(&blob, &other, code, 1_001)
            .unwrap_err()
            .to_string()
            .contains("another credential"));
        // Swapping the named credential breaks the AEAD even for a caller that does not check it.
        let mut forged = blob.clone();
        forged.host_uid = Some(other);
        assert!(snapshot_open(&forged, code, 1_001).is_err());
        // A version 1 blob stays readable and unbound.
        let v1 = snapshot_seal_with_profile(&share(), code, 1_000, CHEAP).unwrap();
        assert_eq!(v1.to_bytes()[4], 1);
        assert_eq!(snapshot_open_for(&v1, &other, code, 1_001).unwrap(), share());
        assert!(SnapshotBlob::from_bytes(&wire[..50]).is_err());
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
