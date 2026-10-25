//! The only place that knows the `ml-kem` crate API. Sizes (FIPS 203, ML-KEM-768):
//! encapsulation key 1184, decapsulation key 2400, ciphertext 1088.

use anyhow::{anyhow, Result};
use ml_kem::{
    kem::{Decapsulate, DecapsulationKey, Encapsulate, EncapsulationKey},
    Encoded, EncodedSizeUser, KemCore, MlKem768, MlKem768Params,
};
use rand_core::{CryptoRng, RngCore};

pub(crate) const EK_LEN: usize = 1184;
pub(crate) const DK_LEN: usize = 2400;
pub(crate) const CT_LEN: usize = 1088;

type Dk = DecapsulationKey<MlKem768Params>;
type Ek = EncapsulationKey<MlKem768Params>;

fn parse_ek(ek: &[u8]) -> Result<Ek> {
    let enc = <Encoded<Ek>>::try_from(ek)
        .map_err(|_| anyhow!("malformed ML-KEM-768 encapsulation key"))?;
    Ok(Ek::from_bytes(&enc))
}

fn parse_dk(dk: &[u8]) -> Result<Dk> {
    let enc = <Encoded<Dk>>::try_from(dk)
        .map_err(|_| anyhow!("malformed ML-KEM-768 decapsulation key"))?;
    Ok(Dk::from_bytes(&enc))
}

/// Fresh key pair: `(encapsulation key, decapsulation key)` as encoded bytes.
pub(crate) fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> (Vec<u8>, Vec<u8>) {
    let (dk, ek) = MlKem768::generate(rng);
    (ek.as_bytes().to_vec(), dk.as_bytes().to_vec())
}

/// FIPS 203 key generation from the seeds `d` and `z`.
pub(crate) fn generate_deterministic(d: &[u8; 32], z: &[u8; 32]) -> (Vec<u8>, Vec<u8>) {
    let d = ml_kem::B32::try_from(&d[..]).expect("32 bytes");
    let z = ml_kem::B32::try_from(&z[..]).expect("32 bytes");
    let (dk, ek) = MlKem768::generate_deterministic(&d, &z);
    (ek.as_bytes().to_vec(), dk.as_bytes().to_vec())
}

/// Checks that bytes are a well-formed decapsulation key and returns its encapsulation key.
pub(crate) fn ek_of_dk(dk: &[u8]) -> Result<Vec<u8>> {
    Ok(parse_dk(dk)?.encapsulation_key().as_bytes().to_vec())
}

/// Checks that bytes are a well-formed encapsulation key.
pub(crate) fn validate_ek(ek: &[u8]) -> Result<()> {
    parse_ek(ek).map(|_| ())
}

/// `(ciphertext, shared secret)`.
pub(crate) fn encapsulate<R: RngCore + CryptoRng>(
    ek: &[u8],
    rng: &mut R,
) -> Result<(Vec<u8>, [u8; 32])> {
    let (ct, ss) = parse_ek(ek)?
        .encapsulate(rng)
        .map_err(|e| anyhow!("ml-kem encapsulate: {e:?}"))?;
    Ok((
        ct.as_slice().to_vec(),
        ss.as_slice()
            .try_into()
            .map_err(|_| anyhow!("bad shared secret length"))?,
    ))
}

#[cfg(test)]
/// FIPS 203 encapsulation with the message randomness `m` given (test vectors).
pub(crate) fn encapsulate_with(ek: &[u8], m: &[u8; 32]) -> Result<(Vec<u8>, [u8; 32])> {
    use ml_kem::EncapsulateDeterministic;
    let m = ml_kem::B32::try_from(&m[..]).map_err(|_| anyhow!("bad m length"))?;
    let (ct, ss) = parse_ek(ek)?
        .encapsulate_deterministic(&m)
        .map_err(|e| anyhow!("ml-kem encapsulate_deterministic: {e:?}"))?;
    Ok((
        ct.as_slice().to_vec(),
        ss.as_slice()
            .try_into()
            .map_err(|_| anyhow!("bad shared secret length"))?,
    ))
}

pub(crate) fn decapsulate(dk: &[u8], ct: &[u8]) -> Result<[u8; 32]> {
    let ct = ml_kem::Ciphertext::<MlKem768>::try_from(ct)
        .map_err(|_| anyhow!("malformed ML-KEM-768 ciphertext"))?;
    let ss = parse_dk(dk)?
        .decapsulate(&ct)
        .map_err(|e| anyhow!("ml-kem decapsulate: {e:?}"))?;
    ss.as_slice()
        .try_into()
        .map_err(|_| anyhow!("bad shared secret length"))
}
