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
use crate::sync::limits::ARGON2_DEVICE_KEY_PROFILE;

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

pub struct DeviceIdentity {
    signing: SigningKey,
    x_secret: XSecret,
    mlkem_dk: Zeroizing<Vec<u8>>,
    mlkem_pub: Vec<u8>,
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
        let mut x = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut ed[..]);
        OsRng.fill_bytes(&mut x[..]);
        let (ek, dk) = mlkem::generate(&mut OsRng);
        Self::assemble(&ed, &x, dk, ek)
    }

    /// Deterministic identity from 32 bytes of key material and a context string:
    /// `BLAKE3 derive_key(context, ikm)` as 128 bytes of output, split into the
    /// Ed25519 seed, the X25519 secret and the ML-KEM seeds `d` and `z`.
    pub(crate) fn from_seed(ikm: &[u8; 32], context: &str) -> Self {
        let mut xof = Zeroizing::new([0u8; 128]);
        derive_xof(context, ikm, &mut xof[..]);
        let (ek, dk) = mlkem::generate_deterministic(
            xof[64..96].try_into().unwrap(),
            xof[96..128].try_into().unwrap(),
        );
        Self::assemble(
            xof[..32].try_into().unwrap(),
            xof[32..64].try_into().unwrap(),
            dk,
            ek,
        )
    }

    fn assemble(ed_seed: &[u8; 32], x_secret: &[u8; 32], dk: Vec<u8>, ek: Vec<u8>) -> Self {
        Self {
            signing: SigningKey::from_bytes(ed_seed),
            x_secret: XSecret::from(*x_secret),
            mlkem_dk: Zeroizing::new(dk),
            mlkem_pub: ek,
        }
    }

    pub fn public(&self) -> DevicePublic {
        DevicePublic {
            ed25519_pub: self.signing.verifying_key().to_bytes(),
            x25519_pub: XPublic::from(&self.x_secret).to_bytes(),
            mlkem_pub: self.mlkem_pub.clone(),
        }
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.signing.sign(msg).to_bytes()
    }

    pub(crate) fn x_secret_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.x_secret.to_bytes())
    }

    pub(crate) fn mlkem_dk_bytes(&self) -> &[u8] {
        &self.mlkem_dk
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

    pub(crate) fn seal_to_with(
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
        if ciphertext.len() < 32 + 24 + 16 {
            bail!("sealed box too short: {} bytes", ciphertext.len());
        }
        let eph_pub: [u8; 32] = ciphertext[..32].try_into().unwrap();
        let nonce = &ciphertext[32..56];
        let dh = self.x_secret.diffie_hellman(&XPublic::from(eph_pub));
        if !dh.was_contributory() {
            bail!("sealed box with a low-order ephemeral key");
        }
        let my_pub = XPublic::from(&self.x_secret).to_bytes();
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

    /// Key file form: `"SSHK" || 1 || mem u32 BE || iters u32 BE || salt(16) || nonce(24) || ct`;
    /// the plaintext is `ed25519 seed || x25519 secret || ML-KEM decapsulation key` and the
    /// whole header is associated data. The key comes from Argon2id over the passphrase.
    pub fn to_encrypted(&self, passphrase: &str) -> Vec<u8> {
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce);
        self.to_encrypted_with(passphrase, ARGON2_DEVICE_KEY_PROFILE, &salt, &nonce)
    }

    pub(crate) fn to_encrypted_with(
        &self,
        passphrase: &str,
        profile: Argon2Profile,
        salt: &[u8; 16],
        nonce: &[u8; 24],
    ) -> Vec<u8> {
        let key = Zeroizing::new(
            try_argon2_derive(passphrase, salt, profile).expect("profile within bounds"),
        );
        let mut plain = Zeroizing::new(Vec::with_capacity(64 + mlkem::DK_LEN));
        plain.extend_from_slice(&self.signing.to_bytes());
        plain.extend_from_slice(&self.x_secret.to_bytes());
        plain.extend_from_slice(&self.mlkem_dk);
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
        if plain.len() != 64 + mlkem::DK_LEN {
            bail!("device key file has an unexpected payload length");
        }
        let ek = mlkem::ek_of_dk(&plain[64..])?;
        Ok(Self::assemble(
            plain[..32].try_into().unwrap(),
            plain[32..64].try_into().unwrap(),
            plain[64..].to_vec(),
            ek,
        ))
    }
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
}
