//! Shamir k-of-n over an in-repo GF(2^8) (stage-0 Task 6) plus proactive
//! resharing without secret reconstruction (Task 11).
//!
//! Field: polynomial 0x11b (AES), multiplicative generator 3, addition = XOR,
//! multiplication via log/exp tables.

use std::sync::OnceLock;

use anyhow::anyhow;
use rand_core::{OsRng, RngCore};

// ---------------------------------------------------------------- GF(2^8)

fn gf_tables() -> &'static ([u8; 256], [u8; 512]) {
    static T: OnceLock<([u8; 256], [u8; 512])> = OnceLock::new();
    T.get_or_init(|| {
        let mut log = [0u8; 256];
        let mut exp = [0u8; 512];
        let mut x: u8 = 1;
        for i in 0..255usize {
            exp[i] = x;
            log[x as usize] = i as u8;
            // x *= 3  ==  x ^ xtime(x)
            let hi = x & 0x80 != 0;
            let mut t = x << 1;
            if hi {
                t ^= 0x1b;
            }
            x = t ^ x;
        }
        for i in 255..512usize {
            exp[i] = exp[i - 255];
        }
        (log, exp)
    })
}

fn gf_mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let (log, exp) = gf_tables();
    exp[log[a as usize] as usize + log[b as usize] as usize]
}

fn gf_inv(a: u8) -> u8 {
    debug_assert!(a != 0, "0 has no inverse in GF(2^8)");
    let (log, exp) = gf_tables();
    exp[255 - log[a as usize] as usize]
}

/// Lagrange coefficients at zero for a set of x-coordinates. In GF(2^8) there
/// is no sign, so (0 - xj) == xj.
fn lagrange_at_zero(xs: &[u8]) -> Vec<u8> {
    xs.iter()
        .enumerate()
        .map(|(i, &xi)| {
            let mut num = 1u8;
            let mut den = 1u8;
            for (j, &xj) in xs.iter().enumerate() {
                if i == j {
                    continue;
                }
                num = gf_mul(num, xj);
                den = gf_mul(den, xi ^ xj);
            }
            gf_mul(num, gf_inv(den))
        })
        .collect()
}

// ---------------------------------------------------------------- Shamir

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    pub index: u8,
    pub bytes: Vec<u8>,
}

/// Split a secret under EXPLICITLY given x-coordinates. Resharing (below)
/// needs this: the new holder set is given by indices, not by length.
fn split_at_indices_with_rng(
    secret: &[u8; 32],
    k: u8,
    indices: &[u8],
    rng: &mut impl RngCore,
) -> Vec<Share> {
    assert!(k >= 2, "threshold below 2 is not secret sharing");
    assert!(indices.iter().all(|&x| x != 0), "x=0 is the secret itself");
    // k-1 random coefficients per byte; constant term = secret.
    let mut coeffs: Vec<[u8; 32]> = Vec::with_capacity(k as usize);
    coeffs.push(*secret);
    for _ in 1..k {
        let mut c = [0u8; 32];
        rng.fill_bytes(&mut c);
        coeffs.push(c);
    }
    indices
        .iter()
        .map(|&x| {
            let mut bytes = [0u8; 32];
            for (b, out) in bytes.iter_mut().enumerate() {
                // Horner, top down.
                let mut acc = 0u8;
                for d in (0..k as usize).rev() {
                    acc = gf_mul(acc, x) ^ coeffs[d][b];
                }
                *out = acc;
            }
            Share {
                index: x,
                bytes: bytes.to_vec(),
            }
        })
        .collect()
}

fn split_at_indices(secret: &[u8; 32], k: u8, indices: &[u8]) -> Vec<Share> {
    split_at_indices_with_rng(secret, k, indices, &mut OsRng)
}

/// Split a 32-byte secret into `n` shares with threshold `k`; x = 1..=n.
pub fn shamir_split(secret: &[u8; 32], k: u8, n: u8) -> Vec<Share> {
    assert!(n >= k && n <= 255, "need k <= n <= 255");
    let indices: Vec<u8> = (1..=n).collect();
    split_at_indices(secret, k, &indices)
}

/// Deterministic split for the vector rig: same `(secret, k, n, rng-state)`
/// gives the same shares. Production code uses [`shamir_split`] (OsRng).
pub fn shamir_split_seeded(secret: &[u8; 32], k: u8, n: u8, rng: &mut impl RngCore) -> Vec<Share> {
    assert!(n >= k && n <= 255, "need k <= n <= 255");
    let indices: Vec<u8> = (1..=n).collect();
    split_at_indices_with_rng(secret, k, &indices, rng)
}

/// Polynomial value at zero for exactly the subset passed in. The threshold is
/// not recorded in a share: a subset smaller than k yields an unrelated value,
/// which the AEAD layer above catches (stage 3 `assemble_dek`).
pub fn shamir_combine(shares: &[Share]) -> anyhow::Result<[u8; 32]> {
    if shares.len() < 2 {
        return Err(anyhow!("need at least 2 shares (k=2 is the product minimum)"));
    }
    if shares.iter().any(|s| s.bytes.len() != 32) {
        return Err(anyhow!("malformed share: expected 32 bytes"));
    }
    if shares.iter().any(|s| s.index == 0) {
        return Err(anyhow!("share index 0 is the secret itself"));
    }
    let xs: Vec<u8> = shares.iter().map(|s| s.index).collect();
    let mut uniq = xs.clone();
    uniq.sort_unstable();
    uniq.dedup();
    if uniq.len() != xs.len() {
        return Err(anyhow!("duplicate share index"));
    }
    let l = lagrange_at_zero(&xs);
    let mut out = [0u8; 32];
    for (b, o) in out.iter_mut().enumerate() {
        let mut acc = 0u8;
        for (i, s) in shares.iter().enumerate() {
            acc ^= gf_mul(l[i], s.bytes[b]);
        }
        *o = acc;
    }
    Ok(out)
}

// ---------------------------------------------------------------- Reshare

/// One acting holder's contribution to a reissue: its own share, re-split
/// under the new holder set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReshareContribution {
    pub from: u8,
    pub sub_shares: Vec<(u8, Vec<u8>)>,
}

/// Re-split YOUR share under the new holder set. The ring secret is not
/// involved: the holder does not know it and cannot learn it here.
pub fn reshare_contribute(my: &Share, new_indices: &[u8], k: u8) -> ReshareContribution {
    reshare_contribute_with_rng(my, new_indices, k, &mut OsRng)
}

/// Deterministic variant for the vector rig.
pub fn reshare_contribute_with_rng(
    my: &Share,
    new_indices: &[u8],
    k: u8,
    rng: &mut impl RngCore,
) -> ReshareContribution {
    let mut mine = [0u8; 32];
    mine.copy_from_slice(&my.bytes);
    let sub = split_at_indices_with_rng(&mine, k, new_indices, rng);
    ReshareContribution {
        from: my.index,
        sub_shares: sub.into_iter().map(|s| (s.index, s.bytes)).collect(),
    }
}

/// Assemble your NEW share from the contributors' contributions.
pub fn reshare_combine(
    my_index: u8,
    contributions: &[ReshareContribution],
) -> anyhow::Result<Share> {
    if contributions.len() < 2 {
        return Err(anyhow!("resharing needs at least k=2 contributors"));
    }
    let xs: Vec<u8> = contributions.iter().map(|c| c.from).collect();
    let mut uniq = xs.clone();
    uniq.sort_unstable();
    uniq.dedup();
    if uniq.len() != xs.len() {
        return Err(anyhow!("duplicate contributor index"));
    }
    let l = lagrange_at_zero(&xs);
    let mut bytes = [0u8; 32];
    for (i, c) in contributions.iter().enumerate() {
        let (_, sub) = c
            .sub_shares
            .iter()
            .find(|(x, _)| *x == my_index)
            .ok_or_else(|| {
                anyhow!(
                    "contribution from {} carries no sub-share for {my_index}",
                    c.from
                )
            })?;
        if sub.len() != 32 {
            return Err(anyhow!("malformed sub-share from {}", c.from));
        }
        for (b, out) in bytes.iter_mut().enumerate() {
            *out ^= gf_mul(l[i], sub[b]);
        }
    }
    Ok(Share {
        index: my_index,
        bytes: bytes.to_vec(),
    })
}
