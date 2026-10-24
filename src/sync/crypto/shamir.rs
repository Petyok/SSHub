//! Shamir secret sharing over GF(2^8) (AES polynomial 0x11b) on a 32-byte secret,
//! and resharing: new shares are computed from old ones without ever
//! reconstructing the secret. In-repo on purpose: the arithmetic is about sixty
//! lines, every vetted crate pulls in an older `rand` generation, and the browser
//! needs the same field.
//!
//! Field arithmetic is branch-free and table-free, so secret bytes do not steer
//! memory access or control flow. Share indices (x coordinates) are public.
//!
//! Resharing is the Herzberg-style re-split of each contributor's own share under a
//! fresh polynomial for the new holder set. Without a prime field there is no Feldman
//! commitment, so `commitments` here are BLAKE3 commitments to each sub-share: they
//! detect substitution and corruption in transit and attribute a bad sub-share to its
//! contributor, but they cannot prove that a contributor's polynomial has the right
//! constant term. That trade (denial of service, not disclosure) is guarded by the
//! caller adopting a reshared set only after one real unlock succeeds under it.

use anyhow::{anyhow, bail, Result};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroize;

use super::hash::derive_key;

const COMMIT_CTX: &str = "sshub/sync/reshare/commit/v1";

/// One holder's share: an x coordinate and the 32 polynomial values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    pub index: u8,
    pub bytes: Vec<u8>,
}

impl Drop for Share {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

/// One contributor's re-split of its own share under the new holder indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReshareContribution {
    pub from: u8,
    pub sub_shares: Vec<(u8, Vec<u8>)>,
    /// One commitment per sub-share, in the same order.
    pub commitments: Vec<Vec<u8>>,
}

// ---------------------------------------------------------------- GF(2^8)

fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut r = 0u8;
    for _ in 0..8 {
        r ^= a & 0u8.wrapping_sub(b & 1);
        let hi = 0u8.wrapping_sub(a >> 7);
        a = (a << 1) ^ (0x1b & hi);
        b >>= 1;
    }
    r
}

/// `a^254`, the inverse of a nonzero element (0 maps to 0).
fn gf_inv(a: u8) -> u8 {
    let a2 = gf_mul(a, a);
    let a3 = gf_mul(a2, a);
    let a6 = gf_mul(a3, a3);
    let a12 = gf_mul(a6, a6);
    let a15 = gf_mul(a12, a3);
    let a30 = gf_mul(a15, a15);
    let a60 = gf_mul(a30, a30);
    let a120 = gf_mul(a60, a60);
    let a240 = gf_mul(a120, a120);
    let a252 = gf_mul(a240, a12);
    gf_mul(a252, a2)
}

/// Lagrange coefficients at zero. In characteristic 2 there is no sign: `0 - xj == xj`.
fn lagrange_at_zero(xs: &[u8]) -> Vec<u8> {
    xs.iter()
        .enumerate()
        .map(|(i, &xi)| {
            let (mut num, mut den) = (1u8, 1u8);
            for (j, &xj) in xs.iter().enumerate() {
                if i != j {
                    num = gf_mul(num, xj);
                    den = gf_mul(den, xi ^ xj);
                }
            }
            gf_mul(num, gf_inv(den))
        })
        .collect()
}

fn check_indices(indices: &[u8], what: &str) -> Result<()> {
    if indices.is_empty() {
        bail!("{what}: no indices");
    }
    if indices.contains(&0) {
        bail!("{what}: index 0 is the secret itself");
    }
    let mut sorted = indices.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() != indices.len() {
        bail!("{what}: duplicate index");
    }
    Ok(())
}

// ---------------------------------------------------------------- Shamir

/// Splits `secret` at explicit x coordinates; the `k - 1` random coefficient rows come from `rng`.
pub(crate) fn split_at_indices(
    secret: &[u8; 32],
    k: u8,
    indices: &[u8],
    rng: &mut impl RngCore,
) -> Vec<Share> {
    assert!(k >= 2, "threshold below 2 is not secret sharing");
    check_indices(indices, "split").expect("valid share indices");
    assert!(
        indices.len() >= k as usize,
        "fewer holders than the threshold"
    );
    let mut coeffs: Vec<[u8; 32]> = vec![*secret];
    for _ in 1..k {
        let mut c = [0u8; 32];
        rng.fill_bytes(&mut c);
        coeffs.push(c);
    }
    let shares = indices
        .iter()
        .map(|&x| {
            let mut bytes = vec![0u8; 32];
            for (b, out) in bytes.iter_mut().enumerate() {
                let mut acc = 0u8;
                for c in coeffs.iter().rev() {
                    acc = gf_mul(acc, x) ^ c[b];
                }
                *out = acc;
            }
            Share { index: x, bytes }
        })
        .collect();
    for c in coeffs.iter_mut() {
        c.zeroize();
    }
    shares
}

/// Splits a 32-byte secret into `n` shares at x = 1..=n with threshold `k`.
pub fn shamir_split(secret: &[u8; 32], k: u8, n: u8) -> Vec<Share> {
    assert!(n >= k, "need k <= n");
    let indices: Vec<u8> = (1..=n).collect();
    split_at_indices(secret, k, &indices, &mut OsRng)
}

/// Splits at explicit x coordinates (the current holder indices need not be 1..=n after a
/// revocation). Panics on index 0, duplicates, or fewer indices than `k`.
pub fn shamir_split_at(secret: &[u8; 32], k: u8, indices: &[u8]) -> Vec<Share> {
    split_at_indices(secret, k, indices, &mut OsRng)
}

/// The polynomial's value at zero for exactly the subset given. The threshold is not
/// recorded in a share: a subset smaller than k yields an unrelated value, which the AEAD
/// layer above (credential decryption) rejects. Only the requester calls this, in memory.
pub fn shamir_combine(shares: &[Share]) -> Result<[u8; 32]> {
    if shares.len() < 2 {
        bail!("need at least 2 shares (k = 2 is the product minimum)");
    }
    if shares.iter().any(|s| s.bytes.len() != 32) {
        bail!("malformed share: expected 32 bytes");
    }
    let xs: Vec<u8> = shares.iter().map(|s| s.index).collect();
    check_indices(&xs, "combine")?;
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

// ---------------------------------------------------------------- resharing

fn commit(from: u8, index: u8, sub_share: &[u8]) -> Vec<u8> {
    let mut m = vec![from, index];
    m.extend_from_slice(sub_share);
    derive_key(COMMIT_CTX, &m).to_vec()
}

pub(crate) fn reshare_contribute_with(
    my: &Share,
    new_indices: &[u8],
    k: u8,
    rng: &mut impl RngCore,
) -> ReshareContribution {
    let mut mine = [0u8; 32];
    mine.copy_from_slice(&my.bytes);
    let sub = split_at_indices(&mine, k, new_indices, rng);
    mine.zeroize();
    ReshareContribution {
        from: my.index,
        commitments: sub
            .iter()
            .map(|s| commit(my.index, s.index, &s.bytes))
            .collect(),
        sub_shares: sub.iter().map(|s| (s.index, s.bytes.clone())).collect(),
    }
}

/// Re-splits THIS holder's share under the new holder indices. Panics on programmer error
/// (index 0, duplicates, fewer new holders than `k`); indices come from the validated device list.
pub fn reshare_contribute(my: &Share, new_indices: &[u8], k: u8) -> ReshareContribution {
    reshare_contribute_with(my, new_indices, k, &mut OsRng)
}

/// Checks the sub-share addressed to `my_index`: present, 32 bytes, matching its commitment.
pub fn reshare_verify(c: &ReshareContribution, my_index: u8) -> Result<()> {
    if c.sub_shares.len() != c.commitments.len() {
        bail!(
            "contribution from {} has {} sub-shares but {} commitments",
            c.from,
            c.sub_shares.len(),
            c.commitments.len()
        );
    }
    let pos = c
        .sub_shares
        .iter()
        .position(|(x, _)| *x == my_index)
        .ok_or_else(|| {
            anyhow!(
                "contribution from {} carries no sub-share for {my_index}",
                c.from
            )
        })?;
    let (x, bytes) = &c.sub_shares[pos];
    if bytes.len() != 32 {
        bail!("malformed sub-share from {}", c.from);
    }
    if c.commitments[pos] != commit(c.from, *x, bytes) {
        bail!("sub-share from {} does not match its commitment", c.from);
    }
    Ok(())
}

/// Assembles this holder's NEW share from the contributors' sub-shares. Needs at least two
/// contributions; the caller passes exactly the contributors it intends to rely on (at least k).
pub fn reshare_combine(my_index: u8, contributions: &[ReshareContribution]) -> Result<Share> {
    if contributions.len() < 2 {
        bail!("resharing needs at least k = 2 contributors");
    }
    let xs: Vec<u8> = contributions.iter().map(|c| c.from).collect();
    check_indices(&xs, "contributors")?;
    let l = lagrange_at_zero(&xs);
    let mut bytes = vec![0u8; 32];
    for (i, c) in contributions.iter().enumerate() {
        reshare_verify(c, my_index)?;
        let (_, sub) = c
            .sub_shares
            .iter()
            .find(|(x, _)| *x == my_index)
            .expect("verified above");
        for (b, out) in bytes.iter_mut().enumerate() {
            *out ^= gf_mul(l[i], sub[b]);
        }
    }
    Ok(Share {
        index: my_index,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::crypto::{dek_open, dek_seal};
    use rand_chacha::{rand_core::SeedableRng, ChaCha20Rng};

    fn rng(seed: u8) -> ChaCha20Rng {
        ChaCha20Rng::from_seed([seed; 32])
    }

    fn field_ok() {
        // exhaustive: inverse and distributivity over the whole field
        for a in 1..=255u8 {
            assert_eq!(gf_mul(a, gf_inv(a)), 1, "inverse of {a}");
        }
        assert_eq!(gf_inv(0), 0);
        for a in [0u8, 1, 2, 0x53, 0xca, 0xff] {
            for b in 0..=255u8 {
                for c in [0u8, 1, 0x1b, 0x80, 0xfe] {
                    assert_eq!(gf_mul(a, b ^ c), gf_mul(a, b) ^ gf_mul(a, c));
                }
            }
        }
        // AES test vector: 0x57 * 0x83 = 0xc1
        assert_eq!(gf_mul(0x57, 0x83), 0xc1);
    }

    #[test]
    fn field_is_the_aes_field() {
        field_ok();
    }

    #[test]
    fn split_and_combine_every_subset() {
        let secret = [0x77u8; 32];
        for (k, n) in [(2u8, 3u8), (2, 5), (3, 5), (3, 6)] {
            let shares =
                split_at_indices(&secret, k, &(1..=n).collect::<Vec<_>>(), &mut rng(k + n));
            // every k-subset recovers
            let idx: Vec<usize> = (0..n as usize).collect();
            let mut count = 0;
            for mask in 0u32..(1 << n) {
                if mask.count_ones() as u8 != k {
                    continue;
                }
                let subset: Vec<Share> = idx
                    .iter()
                    .filter(|i| mask & (1 << **i) != 0)
                    .map(|i| shares[*i].clone())
                    .collect();
                assert_eq!(
                    shamir_combine(&subset).unwrap(),
                    secret,
                    "k={k} n={n} mask={mask:b}"
                );
                count += 1;
            }
            assert!(count > 0);
            // fewer than k does not
            let few: Vec<Share> = shares[..(k - 1) as usize].to_vec();
            if few.len() >= 2 {
                assert_ne!(shamir_combine(&few).unwrap(), secret);
            }
        }
    }

    #[test]
    fn combine_rejects_malformed_input() {
        let s = shamir_split(&[1; 32], 2, 3);
        assert!(shamir_combine(&s[..1]).is_err());
        assert!(
            shamir_combine(&[s[0].clone(), s[0].clone()]).is_err(),
            "duplicate index"
        );
        let mut zero = s[1].clone();
        zero.index = 0;
        assert!(shamir_combine(&[s[0].clone(), zero]).is_err(), "index 0");
        let mut short = s[1].clone();
        short.bytes.truncate(31);
        assert!(shamir_combine(&[s[0].clone(), short]).is_err());
    }

    /// Splits a fresh DEK, seals a credential, and returns everything a test needs.
    fn credential(seed: u8, k: u8, indices: &[u8]) -> ([u8; 32], Vec<u8>, Vec<Share>) {
        let mut r = rng(seed);
        let mut dek = [0u8; 32];
        r.fill_bytes(&mut dek);
        let ct = dek_seal(&dek, b"correct horse battery staple");
        let shares = split_at_indices(&dek, k, indices, &mut r);
        (dek, ct, shares)
    }

    fn reshare(
        old: &[Share],
        contributors: &[usize],
        new_indices: &[u8],
        k: u8,
        seed: u8,
    ) -> Vec<Share> {
        let mut r = rng(seed);
        let contributions: Vec<_> = contributors
            .iter()
            .map(|&i| reshare_contribute_with(&old[i], new_indices, k, &mut r))
            .collect();
        new_indices
            .iter()
            .map(|&x| reshare_combine(x, &contributions).unwrap())
            .collect()
    }

    /// B1: any two new shares recover the original secret, across add / remove / replace.
    #[test]
    fn b1_reshare_preserves_the_secret_across_holder_set_changes() {
        for (name, old_idx, new_idx) in [
            ("add one", vec![1u8, 2, 3], vec![1u8, 2, 3, 4]),
            ("remove one", vec![1, 2, 3, 4], vec![1, 2, 4]),
            ("replace one", vec![1, 2, 3], vec![1, 2, 5]),
            ("all new", vec![1, 2, 3], vec![7, 8, 9, 10]),
        ] {
            let (dek, ct, old) = credential(3, 2, &old_idx);
            for pair in [[0usize, 1], [0, 2], [1, 2]] {
                let new = reshare(&old, &pair, &new_idx, 2, 9);
                for i in 0..new.len() {
                    for j in i + 1..new.len() {
                        let got = shamir_combine(&[new[i].clone(), new[j].clone()]).unwrap();
                        assert_eq!(got, dek, "{name}: contributors {pair:?} new pair ({i},{j})");
                        assert!(
                            dek_open(&got, &ct).is_ok(),
                            "{name}: ciphertext does not open"
                        );
                    }
                }
            }
        }
    }

    /// B2: an old share combined with a new share opens nothing.
    #[test]
    fn b2_mixed_old_and_new_shares_do_not_open_the_credential() {
        let (_, ct, old) = credential(4, 2, &[1, 2, 3]);
        let new = reshare(&old, &[0, 1], &[4, 5, 6], 2, 11);
        for o in &old {
            for n in &new {
                let got = shamir_combine(&[o.clone(), n.clone()]).unwrap();
                assert!(
                    dek_open(&got, &ct).is_err(),
                    "old {} + new {} opened the credential",
                    o.index,
                    n.index
                );
            }
        }
        // the same holder index in both sets (a kept holder) mixed with a different new index
        let kept = reshare(&old, &[0, 1], &[1, 2, 6], 2, 12);
        for o in &old {
            for n in &kept {
                if o.index != n.index {
                    let got = shamir_combine(&[o.clone(), n.clone()]).unwrap();
                    assert!(
                        dek_open(&got, &ct).is_err(),
                        "old {} + new {} opened the credential",
                        o.index,
                        n.index
                    );
                }
            }
        }
    }

    /// B3: a single contributor's sub-shares recombine to THAT contributor's share, never to the secret.
    #[test]
    fn b3_one_contribution_recombines_to_the_contributors_share_not_the_secret() {
        let (dek, _, old) = credential(5, 2, &[1, 2, 3]);
        let c = reshare_contribute_with(&old[0], &[4, 5, 6], 2, &mut rng(13));
        for (a, b) in [(0usize, 1usize), (0, 2), (1, 2)] {
            let subset = [
                Share {
                    index: c.sub_shares[a].0,
                    bytes: c.sub_shares[a].1.clone(),
                },
                Share {
                    index: c.sub_shares[b].0,
                    bytes: c.sub_shares[b].1.clone(),
                },
            ];
            let back = shamir_combine(&subset).unwrap();
            assert_eq!(
                back.as_slice(),
                old[0].bytes.as_slice(),
                "sub-shares must rebuild the contributor's own share"
            );
            assert_ne!(
                back, dek,
                "a single contribution must not rebuild the secret"
            );
        }
    }

    /// The reshare code path never calls `shamir_combine` (only the requester does, when unlocking).
    #[test]
    fn b3b_reshare_functions_never_call_shamir_combine() {
        let src = include_str!("shamir.rs");
        let start = src
            .find("// ---------------------------------------------------------------- resharing")
            .unwrap();
        let end = src.find("#[cfg(test)]").unwrap();
        assert!(
            !src[start..end].contains("shamir_combine"),
            "resharing must not reconstruct the secret"
        );
    }

    /// B4: a dropped or damaged contribution leaves the old set working and the new set unadopted.
    #[test]
    fn b4_a_missing_or_damaged_contribution_is_not_adopted() {
        let (dek, ct, old) = credential(6, 2, &[1, 2, 3]);
        let new_idx = [1u8, 2, 4];
        let mut r = rng(14);
        let c0 = reshare_contribute_with(&old[0], &new_idx, 2, &mut r);
        let c1 = reshare_contribute_with(&old[1], &new_idx, 2, &mut r);
        // contributor 2 dropped: one contribution alone cannot form a new share
        assert!(reshare_combine(4, std::slice::from_ref(&c0)).is_err());
        // the old set still opens at every step
        assert_eq!(
            shamir_combine(&[old[0].clone(), old[1].clone()]).unwrap(),
            dek
        );
        // a contribution damaged in transit is caught by its commitment
        let mut bad = c1.clone();
        bad.sub_shares[0].1[0] ^= 1;
        assert!(reshare_verify(&bad, bad.sub_shares[0].0).is_err());
        assert!(reshare_combine(bad.sub_shares[0].0, &[c0.clone(), bad]).is_err());
        // a contributor that lies consistently (commitment matches its garbage) yields a set that
        // does not open the credential: the provisional set is dropped, the old set is kept
        let liar_share = Share {
            index: old[1].index,
            bytes: vec![0xAA; 32],
        };
        let liar = reshare_contribute_with(&liar_share, &new_idx, 2, &mut r);
        let provisional: Vec<Share> = new_idx
            .iter()
            .map(|&x| reshare_combine(x, &[c0.clone(), liar.clone()]).unwrap())
            .collect();
        let got = shamir_combine(&provisional[..2]).unwrap();
        assert!(
            dek_open(&got, &ct).is_err(),
            "a lying contribution produced a set that opens the credential"
        );
        assert!(
            dek_open(
                &shamir_combine(&[old[0].clone(), old[2].clone()]).unwrap(),
                &ct
            )
            .is_ok(),
            "old set must keep working"
        );
    }

    #[test]
    fn reshare_verify_rejects_missing_and_mismatched_entries() {
        let (_, _, old) = credential(7, 2, &[1, 2, 3]);
        let c = reshare_contribute_with(&old[0], &[4, 5], 2, &mut rng(15));
        assert!(reshare_verify(&c, 4).is_ok());
        assert!(reshare_verify(&c, 9).is_err(), "no sub-share for index 9");
        let mut fewer = c.clone();
        fewer.commitments.pop();
        assert!(reshare_verify(&fewer, 4).is_err());
        let other = reshare_contribute_with(&old[1], &[4, 5], 2, &mut rng(15));
        assert!(
            reshare_combine(4, &[c.clone(), c.clone()]).is_err(),
            "duplicate contributor"
        );
        assert!(reshare_combine(4, &[c, other]).is_ok());
    }
}
