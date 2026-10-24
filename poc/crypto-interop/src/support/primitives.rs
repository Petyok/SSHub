//! Stage-0 primitives: `chain_hash`, `DeviceIdentity`/`verify`, sealed boxes,
//! Argon2id profiles, DEK, and the hybrid X25519+ML-KEM-768 wrap (Tasks 1-5,
//! 7, 10). Signatures match the stage-0 plan; `*_with_rng` / `*_seeded`
//! variants exist only so `emit-vectors` is deterministic.

use anyhow::{anyhow, Context};
use chacha20poly1305::{
    aead::{KeyInit as _},
    XChaCha20Poly1305, XNonce,
};
use crypto_box::{
    aead::{Aead, AeadCore, OsRng as BoxOsRng},
    ChaChaBox, PublicKey as BoxPublic, SecretKey as BoxSecret,
};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use ml_kem::{
    kem::{Decapsulate, DecapsulationKey, Encapsulate, EncapsulationKey},
    EncapsulateDeterministic, Encoded, EncodedSizeUser, KemCore, MlKem768, MlKem768Params,
};
use rand_core::{CryptoRng, OsRng, RngCore};
use x25519_dalek::{PublicKey as X25519Public, StaticSecret as X25519Secret};

/// Op-chain and channel-binding hash - BLAKE3, 32 bytes.
pub fn chain_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

// ---------------------------------------------------------------- identity

/// Device identity: signing pair (Ed25519) + encryption pair (X25519) + the
/// ML-KEM-768 decapsulation key. Separate pairs, no ed-to-x conversion.
/// Private material lives in memory; serialisation is out of scope here.
pub struct DeviceIdentity {
    signing: SigningKey,
    x_secret: X25519Secret,
    mlkem_dk: Vec<u8>,
}

/// Public half - what replicates into the `devices` table.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DevicePublic {
    pub ed25519_pub: [u8; 32],
    pub x25519_pub: [u8; 32],
    pub mlkem_pub: Vec<u8>,
}

impl DeviceIdentity {
    /// Fresh identity from the system CSPRNG.
    pub fn generate() -> Self {
        let signing = SigningKey::generate(&mut OsRng);
        let x_secret = X25519Secret::random_from_rng(OsRng);
        let (mlkem_pub, mlkem_dk) = mlkem_generate(&mut OsRng);
        let _ = mlkem_pub;
        Self {
            signing,
            x_secret,
            mlkem_dk,
        }
    }

    /// Rebuild from explicit parts (vector rig / `verify-js-blobs --key`).
    /// `mlkem_dk` must be the encoded decapsulation key (`mlkem_generate`
    /// output); it is validated by parsing.
    pub fn from_parts(
        ed_seed: &[u8; 32],
        x_secret: &[u8; 32],
        mlkem_dk: Vec<u8>,
    ) -> anyhow::Result<Self> {
        mlkem_parse_dk(&mlkem_dk)?; // fail fast on garbage
        Ok(Self {
            signing: SigningKey::from_bytes(ed_seed),
            x_secret: X25519Secret::from(*x_secret),
            mlkem_dk,
        })
    }

    /// Public keys for replication.
    pub fn public(&self) -> DevicePublic {
        DevicePublic {
            ed25519_pub: self.signing.verifying_key().to_bytes(),
            x25519_pub: X25519Public::from(&self.x_secret).to_bytes(),
            mlkem_pub: mlkem_pub_of_dk(&self.mlkem_dk),
        }
    }

    /// Ed25519 signature over a message.
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.signing.sign(msg).to_bytes()
    }

    /// Decrypt a sealed box with our static X25519 key.
    pub fn open_sealed(&self, ciphertext: &[u8]) -> anyhow::Result<Vec<u8>> {
        if ciphertext.len() < 32 + 24 + 16 {
            return Err(anyhow!("sealed box too short: {} bytes", ciphertext.len()));
        }
        let mut eph = [0u8; 32];
        eph.copy_from_slice(&ciphertext[..32]);
        let nonce = &ciphertext[32..56];
        let body = &ciphertext[56..];

        let my_secret = BoxSecret::from(self.x_secret.to_bytes());
        let boxed = ChaChaBox::new(&BoxPublic::from(eph), &my_secret);
        boxed
            .decrypt(nonce.into(), body)
            .map_err(|_| anyhow!("sealed box AEAD decrypt failed"))
    }

    /// Seal `plaintext` to a static X25519 recipient key. One-shot ephemeral
    /// sender pair → anonymous sealed box.
    /// Format: eph_pub(32) ‖ nonce(24) ‖ ciphertext(len+16).
    pub fn seal_to(recipient_x25519: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
        let eph = BoxSecret::generate(&mut BoxOsRng);
        let nonce = ChaChaBox::generate_nonce(&mut BoxOsRng);
        Self::seal_to_parts(recipient_x25519, plaintext, &eph, &nonce)
    }

    /// Deterministic variant for the vector rig (fixed ephemeral + nonce).
    pub fn seal_to_seeded(
        recipient_x25519: &[u8; 32],
        plaintext: &[u8],
        eph_secret: &[u8; 32],
        nonce: &[u8; 24],
    ) -> Vec<u8> {
        let eph = BoxSecret::from(*eph_secret);
        Self::seal_to_parts(recipient_x25519, plaintext, &eph, nonce.into())
    }

    fn seal_to_parts(
        recipient_x25519: &[u8; 32],
        plaintext: &[u8],
        eph: &BoxSecret,
        nonce: &crypto_box::Nonce,
    ) -> Vec<u8> {
        let recipient = BoxPublic::from(*recipient_x25519);
        let eph_pub = eph.public_key().as_bytes().to_owned();
        let boxed = ChaChaBox::new(&recipient, eph);
        let ct = boxed
            .encrypt(nonce, plaintext)
            .expect("XChaCha20-Poly1305 encrypt is infallible for valid inputs");
        let mut out = Vec::with_capacity(32 + 24 + ct.len());
        out.extend_from_slice(&eph_pub);
        out.extend_from_slice(nonce.as_slice());
        out.extend_from_slice(&ct);
        out
    }

    pub(crate) fn x_secret_bytes(&self) -> [u8; 32] {
        self.x_secret.to_bytes()
    }

    pub(crate) fn mlkem_dk_bytes(&self) -> &[u8] {
        &self.mlkem_dk
    }
}

/// Check an Ed25519 signature against a public key.
pub fn verify(ed25519_pub: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(ed25519_pub) else {
        return false;
    };
    let signature = ed25519_dalek::Signature::from_bytes(sig);
    vk.verify(msg, &signature).is_ok()
}

// ---------------------------------------------------------------- DEK

/// Fresh random 32-byte DEK.
pub fn dek_generate() -> [u8; 32] {
    let mut dek = [0u8; 32];
    OsRng.fill_bytes(&mut dek);
    dek
}

/// Seal plaintext under a DEK. Format: nonce(24) ‖ ciphertext(len+16).
pub fn dek_seal(dek: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    dek_seal_with_nonce(dek, plaintext, &nonce)
}

/// Deterministic variant for the vector rig.
pub fn dek_seal_with_nonce(dek: &[u8; 32], plaintext: &[u8], nonce: &[u8; 24]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(dek.into());
    let ct = cipher
        .encrypt(nonce.into(), plaintext)
        .expect("XChaCha20-Poly1305 encrypt is infallible for valid inputs");
    let mut out = Vec::with_capacity(24 + ct.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    out
}

/// Open a DEK ciphertext. Rejects anything shorter than nonce+tag WITHOUT
/// panicking (`len < 40` → Err).
pub fn dek_open(dek: &[u8; 32], ciphertext: &[u8]) -> anyhow::Result<Vec<u8>> {
    if ciphertext.len() < 24 + 16 {
        return Err(anyhow!("dek ciphertext too short: {} bytes", ciphertext.len()));
    }
    let nonce = XNonce::from_slice(&ciphertext[..24]);
    let cipher = XChaCha20Poly1305::new(dek.into());
    cipher
        .decrypt(nonce, &ciphertext[24..])
        .map_err(|_| anyhow!("dek AEAD decrypt failed"))
}

// ---------------------------------------------------------------- Argon2

/// Argon2id parameters as one named value: bare numbers scattered across
/// files are a way to silently weaken protection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Argon2Profile {
    pub mem_kib: u32,
    pub iters: u32,
}

/// Vault master phrase. Computed on a machine, not in the phone's browser tab.
pub const ARGON2_MASTER: Argon2Profile = Argon2Profile {
    mem_kib: 1024 * 1024,
    iters: 1,
};
/// Short-code snapshot. Floor; the profile actually used travels INSIDE the
/// blob (stage 3) and is shown in the TUI.
pub const ARGON2_SNAPSHOT: Argon2Profile = Argon2Profile {
    mem_kib: 256 * 1024,
    iters: 4,
};
/// Email code: ≥100 bits of entropy, memory-hardness not required.
pub const ARGON2_EMAIL: Argon2Profile = Argon2Profile {
    mem_kib: 64 * 1024,
    iters: 3,
};
/// Vector-rig profile: fast (milliseconds), NEVER a production profile.
/// Every vector derived under it records `{"profile":"TEST"}` in params.
pub const ARGON2_TEST: Argon2Profile = Argon2Profile {
    mem_kib: 32 * 1024,
    iters: 1,
};

fn argon2_key(code: &str, salt: &[u8; 16], mem_kib: u32, iters: u32) -> [u8; 32] {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params =
        Params::new(mem_kib, iters, 1, Some(32)).expect("Argon2 params within bounds");
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(code.as_bytes(), salt, &mut out)
        .expect("Argon2 hash_password_into cannot fail with 32-byte output");
    out
}

/// Argon2id: string → 32-byte wrap key under a NAMED profile.
pub fn argon2_derive(input: &str, salt: &[u8; 16], p: Argon2Profile) -> [u8; 32] {
    argon2_key(input, salt, p.mem_kib, p.iters)
}

// ---------------------------------------------------------------- ML-KEM adapters (Task 10)
//
// The ONLY place that knows the `ml-kem` crate API: swapping the crate costs
// twenty lines here, not edits across the module.

/// Generate an ML-KEM-768 pair. Returns (encapsulation_key_bytes,
/// decapsulation_key_bytes). Sizes (FIPS 203, ML-KEM-768): ek = 1184,
/// dk = 2400, ct = 1088.
pub fn mlkem_generate<R: RngCore + CryptoRng>(rng: &mut R) -> (Vec<u8>, Vec<u8>) {
    let (dk, ek) = MlKem768::generate(rng);
    (ek.as_bytes().to_vec(), dk.as_bytes().to_vec())
}

fn mlkem_parse_dk(dk: &[u8]) -> anyhow::Result<()> {
    let enc = <Encoded<DecapsulationKey<MlKem768Params>>>::try_from(dk)
        .map_err(|_| anyhow!("malformed ML-KEM-768 decapsulation key"))?;
    let _ = DecapsulationKey::<MlKem768Params>::from_bytes(&enc);
    Ok(())
}

fn mlkem_pub_of_dk(dk: &[u8]) -> Vec<u8> {
    let enc = <Encoded<DecapsulationKey<MlKem768Params>>>::try_from(dk)
        .expect("decapsulation key already validated");
    let dk = DecapsulationKey::<MlKem768Params>::from_bytes(&enc);
    dk.encapsulation_key().as_bytes().to_vec()
}

/// Encapsulate to an encoded encapsulation key → (ciphertext, shared_secret).
pub fn mlkem_encapsulate<R: RngCore + CryptoRng>(
    ek: &[u8],
    rng: &mut R,
) -> anyhow::Result<(Vec<u8>, [u8; 32])> {
    let ek_enc = <Encoded<EncapsulationKey<MlKem768Params>>>::try_from(ek)
        .map_err(|_| anyhow!("malformed ML-KEM-768 encapsulation key"))
        .context("mlkem_encapsulate")?;
    let ek = EncapsulationKey::<MlKem768Params>::from_bytes(&ek_enc);
    let (ct, ss) = ek.encapsulate(rng).map_err(|e| anyhow!("ml-kem encapsulate: {e:?}"))?;
    let ss: [u8; 32] = ss.as_slice().try_into().map_err(|_| anyhow!("bad ss len"))?;
    Ok((ct.as_slice().to_vec(), ss))
}

/// Decapsulate a ciphertext with an encoded decapsulation key.
pub fn mlkem_decapsulate(dk: &[u8], ct: &[u8]) -> anyhow::Result<[u8; 32]> {
    let dk_enc = <Encoded<DecapsulationKey<MlKem768Params>>>::try_from(dk)
        .map_err(|_| anyhow!("malformed ML-KEM-768 decapsulation key"))?;
    let dk = DecapsulationKey::<MlKem768Params>::from_bytes(&dk_enc);
    let ct = ml_kem::Ciphertext::<MlKem768>::try_from(ct)
        .map_err(|_| anyhow!("malformed ML-KEM-768 ciphertext"))?;
    let ss = dk.decapsulate(&ct).map_err(|e| anyhow!("ml-kem decapsulate: {e:?}"))?;
    ss.as_slice()
        .try_into()
        .map_err(|_| anyhow!("bad ss len"))
}

/// Deterministic pair generation for the vector rig and seed-derived
/// identities (FIPS 203 keygen from recorded `d`/`z`). Returns
/// (ek_encoded, dk_encoded), same order as [`mlkem_generate`].
pub fn mlkem_generate_deterministic(d: &[u8; 32], z: &[u8; 32]) -> (Vec<u8>, Vec<u8>) {
    let d = ml_kem::B32::try_from(&d[..]).expect("d is 32 bytes");
    let z = ml_kem::B32::try_from(&z[..]).expect("z is 32 bytes");
    let (dk, ek) = MlKem768::generate_deterministic(&d, &z);
    (ek.as_bytes().to_vec(), dk.as_bytes().to_vec())
}

/// Encapsulation with recorded message randomness `m` (FIPS 203 Algorithm 17)
/// - the pairing hook for noble's `encapsulate(ek, m)`.
pub fn mlkem_encapsulate_deterministic(
    ek: &[u8],
    m: &[u8; 32],
) -> anyhow::Result<(Vec<u8>, [u8; 32])> {
    let ek_enc = <Encoded<EncapsulationKey<MlKem768Params>>>::try_from(ek)
        .map_err(|_| anyhow!("malformed ML-KEM-768 encapsulation key"))
        .context("mlkem_encapsulate_deterministic")?;
    let ek = EncapsulationKey::<MlKem768Params>::from_bytes(&ek_enc);
    let m = ml_kem::B32::try_from(&m[..]).map_err(|_| anyhow!("bad m len"))?;
    let (ct, ss) = ek
        .encapsulate_deterministic(&m)
        .map_err(|e| anyhow!("ml-kem encapsulate_deterministic: {e:?}"))?;
    let ss: [u8; 32] = ss.as_slice().try_into().map_err(|_| anyhow!("bad ss len"))?;
    Ok((ct.as_slice().to_vec(), ss))
}

// ---------------------------------------------------------------- hybrid wrap

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigSuite {
    Ed25519 = 1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapSuite {
    X25519MlKem768 = 1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlgTag {
    pub sig: SigSuite,
    pub wrap: WrapSuite,
}
pub const ALG_CURRENT: AlgTag = AlgTag {
    sig: SigSuite::Ed25519,
    wrap: WrapSuite::X25519MlKem768,
};

const WRAP_MAGIC: &[u8; 4] = b"SSHW";
const WRAP_KDF_CONTEXT: &str = "sshub/sync/wrap/v1";

/// Wrap long-lived material: the wrapping key comes from BOTH mechanisms, so
/// opening the blob requires breaking both.
///
/// Blob: SSHW(4) ‖ sig_u8 ‖ wrap_u8 ‖ eph_pub(32) ‖ kem_ct_len u16 BE ‖
/// kem_ct ‖ nonce(24) ‖ ct.
pub fn hybrid_wrap(recipient: &DevicePublic, plaintext: &[u8]) -> Vec<u8> {
    let mut eph_secret = [0u8; 32];
    OsRng.fill_bytes(&mut eph_secret);
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    hybrid_wrap_seeded(recipient, plaintext, &eph_secret, &nonce, &mut OsRng)
}

/// Deterministic variant for the vector rig: fixed ephemeral X25519 secret,
/// fixed XChaCha nonce, caller-seeded RNG for ML-KEM encaps. The vector
/// records eph_pub, kem_ct and nonce, so JS reproduces the KDF exactly.
pub fn hybrid_wrap_seeded<R: RngCore + CryptoRng>(
    recipient: &DevicePublic,
    plaintext: &[u8],
    eph_secret: &[u8; 32],
    nonce: &[u8; 24],
    rng: &mut R,
) -> Vec<u8> {
    let (kem_ct, kem_ss) =
        mlkem_encapsulate(&recipient.mlkem_pub, rng).expect("ml-kem encapsulate");
    hybrid_wrap_with_m(recipient, plaintext, eph_secret, nonce, &kem_ct, &kem_ss)
}

/// Fully explicit deterministic variant: the ML-KEM encapsulation randomness
/// `m` is recorded, so the vector needs no RNG state at all.
pub fn hybrid_wrap_with_m(
    recipient: &DevicePublic,
    plaintext: &[u8],
    eph_secret: &[u8; 32],
    nonce: &[u8; 24],
    kem_ct: &[u8],
    kem_ss: &[u8; 32],
) -> Vec<u8> {
    let eph = X25519Secret::from(*eph_secret);
    let eph_pub = X25519Public::from(&eph);
    let recip_x = X25519Public::from(recipient.x25519_pub);
    let x_ss = eph.diffie_hellman(&recip_x).to_bytes();

    let mut ikm = Vec::with_capacity(32 + 32 + 32 + 32);
    ikm.extend_from_slice(&x_ss);
    ikm.extend_from_slice(kem_ss);
    ikm.extend_from_slice(eph_pub.as_bytes());
    ikm.extend_from_slice(&recipient.x25519_pub);
    let key = blake3::derive_key(WRAP_KDF_CONTEXT, &ikm);

    let ct = XChaCha20Poly1305::new((&key).into())
        .encrypt(nonce.into(), plaintext)
        .expect("xchacha encrypt");

    let mut out = Vec::with_capacity(4 + 2 + 32 + 2 + kem_ct.len() + 24 + ct.len());
    out.extend_from_slice(WRAP_MAGIC);
    out.push(ALG_CURRENT.sig as u8);
    out.push(ALG_CURRENT.wrap as u8);
    out.extend_from_slice(eph_pub.as_bytes());
    out.extend_from_slice(&(kem_ct.len() as u16).to_be_bytes());
    out.extend_from_slice(kem_ct);
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    out
}

/// Read the algorithm tag without parsing the rest: ingest must reject blobs
/// of an unknown algorithm generation with a legible error.
pub fn blob_alg(blob: &[u8]) -> anyhow::Result<AlgTag> {
    if blob.len() < 6 || &blob[..4] != WRAP_MAGIC {
        return Err(anyhow!("not a wrap blob"));
    }
    let sig = match blob[4] {
        1 => SigSuite::Ed25519,
        v => return Err(anyhow!("unknown signature suite {v}")),
    };
    let wrap = match blob[5] {
        1 => WrapSuite::X25519MlKem768,
        v => return Err(anyhow!("unknown wrap suite {v}")),
    };
    Ok(AlgTag { sig, wrap })
}

pub fn hybrid_unwrap(id: &DeviceIdentity, blob: &[u8]) -> anyhow::Result<Vec<u8>> {
    let _alg = blob_alg(blob)?;
    let mut p = 6usize;
    let eph_pub: [u8; 32] = blob
        .get(p..p + 32)
        .ok_or_else(|| anyhow!("truncated blob"))?
        .try_into()
        .unwrap();
    p += 32;
    let kem_len = u16::from_be_bytes(
        blob.get(p..p + 2)
            .ok_or_else(|| anyhow!("truncated blob"))?
            .try_into()
            .unwrap(),
    ) as usize;
    p += 2;
    let kem_ct = blob.get(p..p + kem_len).ok_or_else(|| anyhow!("truncated blob"))?;
    p += kem_len;
    let nonce: [u8; 24] = blob
        .get(p..p + 24)
        .ok_or_else(|| anyhow!("truncated blob"))?
        .try_into()
        .unwrap();
    p += 24;
    let ct = blob.get(p..).ok_or_else(|| anyhow!("truncated blob"))?;

    let x_ss = X25519Secret::from(id.x_secret_bytes())
        .diffie_hellman(&X25519Public::from(eph_pub))
        .to_bytes();
    let kem_ss = mlkem_decapsulate(id.mlkem_dk_bytes(), kem_ct)?;

    let mut ikm = Vec::new();
    ikm.extend_from_slice(&x_ss);
    ikm.extend_from_slice(&kem_ss);
    ikm.extend_from_slice(&eph_pub);
    ikm.extend_from_slice(&id.public().x25519_pub);
    let key = blake3::derive_key(WRAP_KDF_CONTEXT, &ikm);

    XChaCha20Poly1305::new((&key).into())
        .decrypt(&nonce.into(), ct)
        .map_err(|_| anyhow!("hybrid unwrap failed: not our blob or tampered"))
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Hybrid wrap → unwrap roundtrip with fully explicit randomness: the same
    /// fixture set as the vector rig (recipient 0x55, eph 0x99, m recorded).
    #[test]
    fn hybrid_wrap_unwrap_roundtrip() {
        let (ek, dk) = mlkem_generate_deterministic(&[0x66u8; 32], &[0x77u8; 32]);
        let recip = DevicePublic {
            ed25519_pub: [0u8; 32],
            x25519_pub: X25519Public::from(&X25519Secret::from([0x55u8; 32])).to_bytes(),
            mlkem_pub: ek,
        };
        let (kem_ct, kem_ss) =
            mlkem_encapsulate_deterministic(&recip.mlkem_pub, &[0xE0u8; 32]).unwrap();
        let pt = [0xEEu8; 64];
        let blob = hybrid_wrap_with_m(&recip, &pt, &[0x99u8; 32], &[0xBBu8; 24], &kem_ct, &kem_ss);
        assert_eq!(&blob[..4], b"SSHW");

        let id = DeviceIdentity::from_parts(&[0x44u8; 32], &[0x55u8; 32], dk).expect("dk valid");
        assert_eq!(hybrid_unwrap(&id, &blob).unwrap(), pt);

        let mut bad = blob.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(hybrid_unwrap(&id, &bad).is_err());
    }

    /// DEK seal with a fixed nonce → open roundtrip; short input rejected.
    #[test]
    fn dek_seal_open_roundtrip() {
        let dek = [7u8; 32];
        let ct = dek_seal_with_nonce(&dek, b"payload", &[0xBBu8; 24]);
        assert_eq!(dek_open(&dek, &ct).unwrap(), b"payload");
        assert!(dek_open(&dek, &[0u8; 39]).is_err());
    }
}
