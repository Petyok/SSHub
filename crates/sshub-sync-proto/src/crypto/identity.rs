//! Device identity: an Ed25519 signing pair, an X25519 pair and an ML-KEM-768
//! pair, all separate (no ed-to-x conversion). Private keys never leave this
//! type except through [`DeviceIdentity::to_encrypted`].

use anyhow::{anyhow, bail, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305,
};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand_core::{OsRng, RngCore};
use x25519_dalek::{PublicKey as XPublic, StaticSecret as XSecret};
use zeroize::Zeroizing;

use super::argon::{try_argon2_derive, Argon2Profile};
use super::hash::{derive_key, derive_xof};
use super::mlkem;
use crate::limits::ARGON2_DEVICE_KEY_PROFILE;

/// The public half of an identity; what replicates into the `devices` table.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DevicePublic {
    pub ed25519_pub: [u8; 32],
    pub x25519_pub: [u8; 32],
    /// ML-KEM-768 encapsulation key (1184 bytes).
    pub mlkem_pub: Vec<u8>,
}

impl DevicePublic {
    /// Structural validation for keys that arrive from another device.
    pub fn validate(&self) -> Result<()> {
        VerifyingKey::from_bytes(&self.ed25519_pub)
            .map_err(|_| anyhow!("invalid Ed25519 public key"))?;
        if self.mlkem_pub.len() != mlkem::EK_LEN {
            bail!(
                "ML-KEM-768 public key must be {} bytes, got {}",
                mlkem::EK_LEN,
                self.mlkem_pub.len()
            );
        }
        mlkem::validate_ek(&self.mlkem_pub)
    }

    /// Short stable id: lowercase hex of the first 8 bytes of the Ed25519 key.
    pub fn device_id(&self) -> String {
        device_id_of(&self.ed25519_pub)
    }
}

/// Short stable device id for an Ed25519 public key.
pub fn device_id_of(ed25519_pub: &[u8; 32]) -> String {
    hex::encode(&ed25519_pub[..8])
}

/// One generation of wrapping keys: an X25519 secret and an ML-KEM-768 decapsulation key.
pub struct WrapKeys {
    x_secret: XSecret,
    mlkem_dk: Zeroizing<Vec<u8>>,
    mlkem_pub: Vec<u8>,
}

impl WrapKeys {
    fn generate() -> Self {
        let mut x = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut x[..]);
        let (ek, dk) = mlkem::generate(&mut OsRng);
        Self {
            x_secret: XSecret::from(*x),
            mlkem_dk: Zeroizing::new(dk),
            mlkem_pub: ek,
        }
    }

    pub(crate) fn x_secret_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.x_secret.to_bytes())
    }

    pub(crate) fn x_public(&self) -> [u8; 32] {
        XPublic::from(&self.x_secret).to_bytes()
    }

    pub(crate) fn mlkem_dk_bytes(&self) -> &[u8] {
        &self.mlkem_dk
    }
}

/// The device's keys: a long-lived signing key and the CURRENT wrapping keys, plus older
/// wrapping generations kept only until [`DeviceIdentity::retire_previous_wrap_keys`].
///
/// Forward secrecy for wrapped material: after a holder rotates its wrapping keys and every
/// credential has been reshared to the new keys, retiring the old generation makes every blob
/// wrapped to it undecryptable, including copies that remain in the append-only log.
pub struct DeviceIdentity {
    signing: SigningKey,
    current: WrapKeys,
    previous: Vec<WrapKeys>,
}

impl std::fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DeviceIdentity({})", self.public().device_id())
    }
}

/// Verifies an Ed25519 signature strictly (RFC 8032: canonical S, no small-order keys).
/// Browsers must call noble with `zip215: false` to agree.
pub fn verify(ed25519_pub: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(ed25519_pub) else {
        return false;
    };
    vk.verify_strict(msg, &ed25519_dalek::Signature::from_bytes(sig))
        .is_ok()
}

const SEAL_CTX: &str = "sshub/sync/seal/v1";
const KEYFILE_MAGIC: &[u8; 4] = b"SSHK";
const KEYFILE_VERSION: u8 = 1;
const KEYFILE_HEADER: usize = 4 + 1 + 4 + 4 + 16 + 24;

impl DeviceIdentity {
    /// A fresh identity from the OS CSPRNG.
    pub fn generate() -> Self {
        let mut ed = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut ed[..]);
        Self {
            signing: SigningKey::from_bytes(&ed),
            current: WrapKeys::generate(),
            previous: Vec::new(),
        }
    }

    /// Deterministic identity from 32 bytes of key material and a context string:
    /// `BLAKE3 derive_key(context, ikm)` as 128 bytes of output, split into the
    /// Ed25519 seed, the X25519 secret and the ML-KEM seeds `d` and `z`.
    #[doc(hidden)]
    pub fn from_seed(ikm: &[u8; 32], context: &str) -> Self {
        let mut xof = Zeroizing::new([0u8; 128]);
        derive_xof(context, ikm, &mut xof[..]);
        let (ek, dk) = mlkem::generate_deterministic(
            xof[64..96].try_into().unwrap(),
            xof[96..128].try_into().unwrap(),
        );
        Self {
            signing: SigningKey::from_bytes(xof[..32].try_into().unwrap()),
            current: WrapKeys {
                x_secret: XSecret::from(<[u8; 32]>::try_from(&xof[32..64]).unwrap()),
                mlkem_dk: Zeroizing::new(dk),
                mlkem_pub: ek,
            },
            previous: Vec::new(),
        }
    }

    /// The public half: signing key plus the CURRENT wrapping keys.
    pub fn public(&self) -> DevicePublic {
        DevicePublic {
            ed25519_pub: self.signing.verifying_key().to_bytes(),
            x25519_pub: self.current.x_public(),
            mlkem_pub: self.current.mlkem_pub.clone(),
        }
    }

    /// Generates fresh wrapping keys and returns the new public half to publish. The previous
    /// generation stays usable for unwrapping blobs wrapped before the rotation became known.
    pub fn rotate_wrap_keys(&mut self) -> DevicePublic {
        let old = std::mem::replace(&mut self.current, WrapKeys::generate());
        self.previous.insert(0, old);
        self.public()
    }

    /// Makes the wrapping keys derived from these seeds the CURRENT generation (the old current one moves to the
    /// previous generations, newest first). For identities whose wrap generations are DERIVED from a secret and a
    /// generation number (the phone), not drawn at random like [`Self::rotate_wrap_keys`].
    #[doc(hidden)]
    pub fn push_wrap_generation(&mut self, x_secret: &[u8; 32], mlkem_d: &[u8; 32], mlkem_z: &[u8; 32]) {
        let (ek, dk) = mlkem::generate_deterministic(mlkem_d, mlkem_z);
        let old = std::mem::replace(
            &mut self.current,
            WrapKeys {
                x_secret: XSecret::from(*x_secret),
                mlkem_dk: Zeroizing::new(dk),
                mlkem_pub: ek,
            },
        );
        self.previous.insert(0, old);
    }

    /// Drops (and zeroizes) every wrapping generation but the current one; returns how many
    /// were dropped. Call only after every credential has been reshared to the current keys.
    pub fn retire_previous_wrap_keys(&mut self) -> usize {
        let n = self.previous.len();
        self.previous.clear();
        n
    }

    /// Number of wrapping generations held (1 = only the current one).
    pub fn wrap_key_generations(&self) -> usize {
        1 + self.previous.len()
    }

    /// Wrapping generations, newest first.
    pub fn wrap_keys(&self) -> impl Iterator<Item = &WrapKeys> {
        std::iter::once(&self.current).chain(self.previous.iter())
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.signing.sign(msg).to_bytes()
    }

    /// Sealed box to a static X25519 key: `eph_pub(32) || nonce(24) || ciphertext`,
    /// XChaCha20-Poly1305 under `BLAKE3 derive_key("sshub/sync/seal/v1", dh || eph_pub ||
    /// recipient_pub)`, with `eph_pub || recipient_pub` as associated data.
    pub fn seal_to(recipient_x25519: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
        let mut eph = Zeroizing::new([0u8; 32]);
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut eph[..]);
        OsRng.fill_bytes(&mut nonce);
        Self::seal_to_with(recipient_x25519, plaintext, &eph, &nonce)
    }

    #[doc(hidden)]
    pub fn seal_to_with(
        recipient_x25519: &[u8; 32],
        plaintext: &[u8],
        eph_secret: &[u8; 32],
        nonce: &[u8; 24],
    ) -> Vec<u8> {
        let eph = XSecret::from(*eph_secret);
        let eph_pub = XPublic::from(&eph).to_bytes();
        let dh = eph.diffie_hellman(&XPublic::from(*recipient_x25519));
        let key = seal_key(dh.as_bytes(), &eph_pub, recipient_x25519);
        let aad = [eph_pub.as_slice(), recipient_x25519.as_slice()].concat();
        let ct = XChaCha20Poly1305::new((&*key).into())
            .encrypt(
                nonce.into(),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .expect("XChaCha20-Poly1305 encrypt");
        [eph_pub.as_slice(), nonce.as_slice(), ct.as_slice()].concat()
    }

    pub fn open_sealed(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let mut last = anyhow!("sealed box authentication failed");
        for keys in self.wrap_keys() {
            match open_sealed_with_secret(&keys.x_secret_bytes(), ciphertext) {
                Ok(plain) => return Ok(plain),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Key file form: `"SSHK" || 1 || mem u32 BE || iters u32 BE || salt(16) || nonce(24) || ct`;
    /// the plaintext is `ed25519 seed || x25519 secret || ML-KEM decapsulation key || n u8 ||
    /// n * (x25519 secret || ML-KEM decapsulation key)` (older wrapping generations) and the
    /// whole header is associated data. The key comes from Argon2id over the passphrase.
    pub fn to_encrypted(&self, passphrase: &str) -> Vec<u8> {
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce);
        self.to_encrypted_with(passphrase, ARGON2_DEVICE_KEY_PROFILE, &salt, &nonce)
    }

    #[doc(hidden)]
    pub fn to_encrypted_with(
        &self,
        passphrase: &str,
        profile: Argon2Profile,
        salt: &[u8; 16],
        nonce: &[u8; 24],
    ) -> Vec<u8> {
        let key = Zeroizing::new(
            try_argon2_derive(passphrase, salt, profile).expect("profile within bounds"),
        );
        let mut plain = Zeroizing::new(Vec::with_capacity(64 + mlkem::DK_LEN + 1));
        plain.extend_from_slice(&self.signing.to_bytes());
        plain.extend_from_slice(&self.current.x_secret.to_bytes());
        plain.extend_from_slice(&self.current.mlkem_dk);
        plain.push(u8::try_from(self.previous.len()).expect("fewer than 256 wrapping generations"));
        for old in &self.previous {
            plain.extend_from_slice(&old.x_secret.to_bytes());
            plain.extend_from_slice(&old.mlkem_dk);
        }
        let mut out = Vec::with_capacity(KEYFILE_HEADER + plain.len() + 16);
        out.extend_from_slice(KEYFILE_MAGIC);
        out.push(KEYFILE_VERSION);
        out.extend_from_slice(&profile.mem_kib.to_be_bytes());
        out.extend_from_slice(&profile.iters.to_be_bytes());
        out.extend_from_slice(salt);
        out.extend_from_slice(nonce);
        let ct = XChaCha20Poly1305::new((&*key).into())
            .encrypt(
                nonce.into(),
                Payload {
                    msg: &plain,
                    aad: &out,
                },
            )
            .expect("XChaCha20-Poly1305 encrypt");
        out.extend_from_slice(&ct);
        out
    }

    pub fn from_encrypted(blob: &[u8], passphrase: &str) -> Result<Self> {
        if blob.len() < KEYFILE_HEADER + 16 || &blob[..4] != KEYFILE_MAGIC {
            bail!("not a device key file");
        }
        if blob[4] != KEYFILE_VERSION {
            bail!("unsupported device key file version {}", blob[4]);
        }
        let profile = Argon2Profile {
            mem_kib: u32::from_be_bytes(blob[5..9].try_into().unwrap()),
            iters: u32::from_be_bytes(blob[9..13].try_into().unwrap()),
        };
        let salt: [u8; 16] = blob[13..29].try_into().unwrap();
        let nonce = &blob[29..53];
        let key = Zeroizing::new(try_argon2_derive(passphrase, &salt, profile)?);
        let plain = Zeroizing::new(
            XChaCha20Poly1305::new((&*key).into())
                .decrypt(
                    nonce.into(),
                    Payload {
                        msg: &blob[KEYFILE_HEADER..],
                        aad: &blob[..KEYFILE_HEADER],
                    },
                )
                .map_err(|_| anyhow!("device key file: wrong passphrase or damaged file"))?,
        );
        const GEN: usize = 32 + mlkem::DK_LEN;
        let bad = || anyhow!("device key file has an unexpected payload length");
        if plain.len() < 32 + GEN + 1 {
            return Err(bad());
        }
        let n_prev = plain[32 + GEN] as usize;
        if plain.len() != 32 + GEN + 1 + n_prev * GEN {
            return Err(bad());
        }
        let wrap_from = |chunk: &[u8]| -> Result<WrapKeys> {
            let dk = chunk[32..].to_vec();
            Ok(WrapKeys {
                x_secret: XSecret::from(<[u8; 32]>::try_from(&chunk[..32]).unwrap()),
                mlkem_pub: mlkem::ek_of_dk(&dk)?,
                mlkem_dk: Zeroizing::new(dk),
            })
        };
        let previous = plain[32 + GEN + 1..]
            .chunks(GEN)
            .map(wrap_from)
            .collect::<Result<Vec<_>>>()?;
        let current = wrap_from(&plain[32..32 + GEN])?;
        Ok(Self {
            signing: SigningKey::from_bytes(plain[..32].try_into().unwrap()),
            current,
            previous,
        })
    }
}

/// Opens a sealed box with a bare X25519 secret (the requester's ephemeral key for a share
/// response, which is not a device identity).
pub fn open_sealed_with_secret(x25519_secret: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.len() < 32 + 24 + 16 {
        bail!("sealed box too short: {} bytes", ciphertext.len());
    }
    let secret = XSecret::from(*x25519_secret);
    let eph_pub: [u8; 32] = ciphertext[..32].try_into().unwrap();
    let nonce = &ciphertext[32..56];
    let dh = secret.diffie_hellman(&XPublic::from(eph_pub));
    if !dh.was_contributory() {
        bail!("sealed box with a low-order ephemeral key");
    }
    let my_pub = XPublic::from(&secret).to_bytes();
    let key = seal_key(dh.as_bytes(), &eph_pub, &my_pub);
    let aad = [eph_pub.as_slice(), my_pub.as_slice()].concat();
    XChaCha20Poly1305::new((&*key).into())
        .decrypt(
            nonce.into(),
            Payload {
                msg: &ciphertext[56..],
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("sealed box authentication failed"))
}

fn seal_key(dh: &[u8; 32], eph_pub: &[u8; 32], recipient_pub: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let ikm =
        Zeroizing::new([dh.as_slice(), eph_pub.as_slice(), recipient_pub.as_slice()].concat());
    Zeroizing::new(derive_key(SEAL_CTX, &ikm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_strictness() {
        let id = DeviceIdentity::generate();
        let sig = id.sign(b"msg");
        let pubk = id.public().ed25519_pub;
        assert!(verify(&pubk, b"msg", &sig));
        assert!(!verify(&pubk, b"other", &sig));
        let mut bad = sig;
        bad[0] ^= 1;
        assert!(!verify(&pubk, b"msg", &bad));
        // S + L is a valid signature under lax rules and must be rejected
        let l: [u8; 32] = [
            0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9,
            0xde, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
        ];
        let mut carry = 0u16;
        let mut mall = sig;
        for i in 0..32 {
            let s = u16::from(sig[32 + i]) + u16::from(l[i]) + carry;
            mall[32 + i] = (s & 0xff) as u8;
            carry = s >> 8;
        }
        if carry == 0 {
            assert!(
                !verify(&pubk, b"msg", &mall),
                "malleated signature accepted"
            );
        }
    }

    #[test]
    fn public_halves_validate_and_ids_are_stable() {
        let id = DeviceIdentity::generate();
        let p = id.public();
        p.validate().unwrap();
        assert_eq!(p.device_id(), hex::encode(&p.ed25519_pub[..8]));
        let mut bad = p.clone();
        bad.mlkem_pub.truncate(10);
        assert!(bad.validate().is_err());
    }

    #[test]
    fn from_seed_is_deterministic_and_context_separated() {
        let a = DeviceIdentity::from_seed(&[5; 32], "sshub/test/a").public();
        assert_eq!(
            a,
            DeviceIdentity::from_seed(&[5; 32], "sshub/test/a").public()
        );
        assert_ne!(
            a.ed25519_pub,
            DeviceIdentity::from_seed(&[5; 32], "sshub/test/b")
                .public()
                .ed25519_pub
        );
        assert_ne!(
            a.ed25519_pub,
            DeviceIdentity::from_seed(&[6; 32], "sshub/test/a")
                .public()
                .ed25519_pub
        );
    }

    #[test]
    fn sealed_box_round_trip_and_rejections() {
        let id = DeviceIdentity::generate();
        let other = DeviceIdentity::generate();
        let pubk = id.public().x25519_pub;
        for pt in [&b""[..], b"share", &[3u8; 500]] {
            assert_eq!(
                id.open_sealed(&DeviceIdentity::seal_to(&pubk, pt)).unwrap(),
                pt
            );
        }
        let ct = DeviceIdentity::seal_to(&pubk, b"share");
        assert!(other.open_sealed(&ct).is_err(), "wrong recipient");
        let mut t = ct.clone();
        let last = t.len() - 1;
        t[last] ^= 1;
        assert!(id.open_sealed(&t).is_err(), "tampered");
        let mut hdr = ct.clone();
        hdr[0] ^= 1;
        assert!(id.open_sealed(&hdr).is_err(), "tampered ephemeral key");
        assert!(id.open_sealed(&ct[..70]).is_err(), "truncated");
        assert!(id.open_sealed(&[0u8; 10]).is_err(), "too short");
        let mut low = ct;
        low[..32].fill(0);
        assert!(id.open_sealed(&low).is_err(), "low-order ephemeral key");
    }

    #[test]
    fn key_file_round_trip_and_rejections() {
        let id = DeviceIdentity::generate();
        let p = Argon2Profile {
            mem_kib: 64,
            iters: 1,
        };
        let blob = id.to_encrypted_with("pw", p, &[1; 16], &[2; 24]);
        let back = DeviceIdentity::from_encrypted(&blob, "pw").unwrap();
        assert_eq!(back.public(), id.public());
        assert_eq!(back.sign(b"x"), id.sign(b"x"));
        assert!(DeviceIdentity::from_encrypted(&blob, "wrong").is_err());
        let mut t = blob.clone();
        t[6] ^= 1; // header is associated data (and the profile): tampering must fail
        assert!(DeviceIdentity::from_encrypted(&t, "pw").is_err());
        let mut big = blob.clone();
        big[5..9].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(
            DeviceIdentity::from_encrypted(&big, "pw").is_err(),
            "memory bomb in header"
        );
        assert!(DeviceIdentity::from_encrypted(&blob[..40], "pw").is_err());
    }

    #[test]
    fn rotation_keeps_old_blobs_readable_until_retired() {
        let mut id = DeviceIdentity::generate();
        let before = id.public();
        let old_blob = crate::crypto::hybrid_wrap(&before, b"old epoch share");
        let old_sealed = DeviceIdentity::seal_to(&before.x25519_pub, b"old sealed");
        let after = id.rotate_wrap_keys();
        assert_eq!(
            after.ed25519_pub, before.ed25519_pub,
            "the signing key does not rotate"
        );
        assert_ne!(after.x25519_pub, before.x25519_pub);
        assert_ne!(after.mlkem_pub, before.mlkem_pub);
        assert_eq!(id.wrap_key_generations(), 2);
        let new_blob = crate::crypto::hybrid_wrap(&after, b"new epoch share");
        assert_eq!(
            crate::crypto::hybrid_unwrap(&id, &old_blob).unwrap(),
            b"old epoch share"
        );
        assert_eq!(
            crate::crypto::hybrid_unwrap(&id, &new_blob).unwrap(),
            b"new epoch share"
        );
        assert_eq!(id.open_sealed(&old_sealed).unwrap(), b"old sealed");
        assert_eq!(id.retire_previous_wrap_keys(), 1);
        assert_eq!(id.wrap_key_generations(), 1);
        assert!(
            crate::crypto::hybrid_unwrap(&id, &old_blob).is_err(),
            "retired generation must not open old blobs"
        );
        assert!(id.open_sealed(&old_sealed).is_err());
        assert_eq!(
            crate::crypto::hybrid_unwrap(&id, &new_blob).unwrap(),
            b"new epoch share"
        );
    }

    #[test]
    fn the_key_file_keeps_previous_generations_across_a_restart() {
        let mut id = DeviceIdentity::generate();
        let old_blob = crate::crypto::hybrid_wrap(&id.public(), b"kept");
        id.rotate_wrap_keys();
        let p = Argon2Profile {
            mem_kib: 64,
            iters: 1,
        };
        let back = DeviceIdentity::from_encrypted(
            &id.to_encrypted_with("pw", p, &[1; 16], &[2; 24]),
            "pw",
        )
        .unwrap();
        assert_eq!(back.wrap_key_generations(), 2);
        assert_eq!(back.public(), id.public());
        assert_eq!(
            crate::crypto::hybrid_unwrap(&back, &old_blob).unwrap(),
            b"kept"
        );
    }
}
