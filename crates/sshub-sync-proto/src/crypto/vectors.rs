//! Golden test vectors for stage 0: `tests/vectors/crypto/vectors.json`.
//!
//! `cargo test` rebuilds every cheap vector and compares it with the committed file.
//! `SSHUB_EMIT_VECTORS=1 cargo test sync::crypto::vectors` rewrites the file (all
//! vectors, including the 1 GiB Argon2 ones). The same file is replayed by the
//! browser implementation and recomputed by the independent Python oracle in
//! `tests/oracles/`. All key material is synthetic (byte patterns).

use serde_json::{json, Value};

use super::argon::{argon2_derive, Argon2Profile, ARGON2_EMAIL, ARGON2_MASTER, ARGON2_SNAPSHOT};
use super::dek::dek_seal_with;
use super::hash::{chain_hash, derive_key, derive_xof};
use super::identity::{verify, DeviceIdentity};
use super::mlkem;
use super::paper::{paper_derive, PaperDomain};
use super::sas::sas_words;
use super::shamir::{
    reshare_combine, reshare_contribute_with, shamir_combine, split_at_indices, Share,
};
use super::vault::vault_recipient_with;
use super::words::{bip39_encode, eff_encode};
use super::wrap::{hybrid_unwrap, hybrid_wrap_with};

const TEST_PROFILE: Argon2Profile = Argon2Profile {
    mem_kib: 32 * 1024,
    iters: 1,
};
const RECIPIENT_CTX: &str = "sshub/vectors/recipient/v1";

fn hx(b: &[u8]) -> String {
    hex::encode(b)
}

fn counting(n: usize, start: u8) -> Vec<u8> {
    (0..n).map(|i| start.wrapping_add(i as u8)).collect()
}

fn vector(
    id: impl Into<String>,
    primitive: &str,
    params: Value,
    inputs: Value,
    outputs: Value,
) -> Value {
    json!({ "id": id.into(), "primitive": primitive, "params": params, "inputs": inputs, "outputs": outputs })
}

fn pubs(p: &super::identity::DevicePublic) -> Value {
    json!({ "ed25519_pub": hx(&p.ed25519_pub), "x25519_pub": hx(&p.x25519_pub), "mlkem_pub": hx(&p.mlkem_pub) })
}

fn profile_json(name: &str, p: Argon2Profile) -> Value {
    json!({ "profile": name, "mem_kib": p.mem_kib, "iters": p.iters })
}

pub(super) fn build(include_slow: bool) -> Vec<Value> {
    let mut v = Vec::new();

    for (id, bytes) in [
        ("chain_hash-empty", Vec::new()),
        ("chain_hash-sshub", b"sshub".to_vec()),
    ] {
        v.push(vector(
            id,
            "chain_hash",
            json!({}),
            json!({ "bytes": hx(&bytes) }),
            json!({ "hash": hx(&chain_hash(&bytes)) }),
        ));
    }
    let ikm = counting(32, 0x20);
    for ctx in [
        "sshub/sync/wrap/v1",
        "sshub/sync/seal/v1",
        "sshub/sync/vault/seed/v1",
        "sshub/sync/vault/v1",
        "sshub/sync/paper/unwrap/v1",
        "sshub/sync/paper/recover/v1",
    ] {
        v.push(vector(
            format!(
                "derive_key-{}",
                ctx.trim_start_matches("sshub/sync/").replace('/', "-")
            ),
            "derive_key",
            json!({ "context": ctx }),
            json!({ "ikm": hx(&ikm) }),
            json!({ "key": hx(&derive_key(ctx, &ikm)) }),
        ));
    }
    let mut xof = [0u8; 128];
    derive_xof("sshub/sync/paper/unwrap/v1", &ikm, &mut xof);
    v.push(vector(
        "derive_xof-128",
        "derive_xof",
        json!({ "context": "sshub/sync/paper/unwrap/v1", "out_len": 128 }),
        json!({ "ikm": hx(&ikm) }),
        json!({ "out": hx(&xof) }),
    ));

    // Ed25519: signing and strict verification.
    let seed = [0x11u8; 32];
    let id = DeviceIdentity::from_seed(&seed, "sshub/vectors/signer/v1");
    let msg = b"sshub/sync/test message".to_vec();
    let sig = id.sign(&msg);
    let p = id.public();
    v.push(vector(
        "ed25519-sign",
        "ed25519_sign",
        json!({ "context": "sshub/vectors/signer/v1" }),
        json!({ "ikm": hx(&seed), "msg": hx(&msg) }),
        json!({ "pub": hx(&p.ed25519_pub), "sig": hx(&sig) }),
    ));
    v.push(vector(
        "ed25519-verify-ok",
        "ed25519_verify",
        json!({}),
        json!({ "pub": hx(&p.ed25519_pub), "msg": hx(&msg), "sig": hx(&sig) }),
        json!({ "valid": verify(&p.ed25519_pub, &msg, &sig) }),
    ));
    let mut flipped = sig;
    flipped[10] ^= 1;
    v.push(vector(
        "ed25519-verify-flipped",
        "ed25519_verify",
        json!({}),
        json!({ "pub": hx(&p.ed25519_pub), "msg": hx(&msg), "sig": hx(&flipped) }),
        json!({ "valid": verify(&p.ed25519_pub, &msg, &flipped) }),
    ));
    // S + L: valid under lax verification, rejected by strict verification.
    let l =
        hex::decode("edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010").unwrap();
    let mut mall = sig;
    let mut carry = 0u16;
    for i in 0..32 {
        let s = u16::from(sig[32 + i]) + u16::from(l[i]) + carry;
        mall[32 + i] = (s & 0xff) as u8;
        carry = s >> 8;
    }
    if carry == 0 {
        v.push(vector(
            "ed25519-verify-noncanonical-s",
            "ed25519_verify",
            json!({ "note": "S + L, rejected by strict verification" }),
            json!({ "pub": hx(&p.ed25519_pub), "msg": hx(&msg), "sig": hx(&mall) }),
            json!({ "valid": verify(&p.ed25519_pub, &msg, &mall) }),
        ));
    }

    // X25519.
    let a_sec = [0x22u8; 32];
    let b_sec = [0x55u8; 32];
    let a_pub = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(a_sec)).to_bytes();
    let b_pub = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(b_sec)).to_bytes();
    let shared = x25519_dalek::StaticSecret::from(a_sec)
        .diffie_hellman(&x25519_dalek::PublicKey::from(b_pub));
    v.push(vector(
        "x25519-dh",
        "x25519_dh",
        json!({ "b_sec": hx(&b_sec) }),
        json!({ "a_sec": hx(&a_sec), "b_pub": hx(&b_pub) }),
        json!({ "a_pub": hx(&a_pub), "shared": hx(shared.as_bytes()) }),
    ));

    // ML-KEM-768.
    let (d, z) = ([0x66u8; 32], [0x77u8; 32]);
    let (ek, dk) = mlkem::generate_deterministic(&d, &z);
    let m = counting(32, 0).try_into().unwrap();
    let (ct, ss) = mlkem::encapsulate_with(&ek, &m).unwrap();
    assert_eq!(mlkem::decapsulate(&dk, &ct).unwrap(), ss);
    v.push(vector(
        "mlkem-encaps",
        "mlkem_encaps",
        json!({ "d": hx(&d), "z": hx(&z), "m": hx(&m) }),
        json!({ "ek": hx(&ek) }),
        json!({ "ct": hx(&ct), "ss": hx(&ss) }),
    ));
    v.push(vector(
        "mlkem-decaps",
        "mlkem_decaps",
        json!({ "d": hx(&d), "z": hx(&z) }),
        json!({ "dk": hx(&dk), "ct": hx(&ct) }),
        json!({ "ss": hx(&ss) }),
    ));

    // DEK seal.
    let dek = [0xD3u8; 32];
    let nonce = [0xBBu8; 24];
    for (name, pt) in [
        ("empty", vec![]),
        ("1", vec![0x77u8]),
        ("40", vec![0x77u8; 40]),
        ("1024", vec![0x77u8; 1024]),
    ] {
        v.push(vector(
            format!("dek_seal-{name}"),
            "dek_seal",
            json!({ "nonce": hx(&nonce) }),
            json!({ "dek": hx(&dek), "plaintext": hx(&pt) }),
            json!({ "ct": hx(&dek_seal_with(&dek, &pt, &nonce)) }),
        ));
    }

    // Argon2id.
    for (name, prof, slow) in [
        ("TEST", TEST_PROFILE, false),
        ("SNAPSHOT", ARGON2_SNAPSHOT, false),
        ("EMAIL", ARGON2_EMAIL, false),
        ("MASTER", ARGON2_MASTER, true),
    ] {
        if slow && !include_slow {
            continue;
        }
        let key = argon2_derive("correct horse battery staple", &[0x5A; 16], prof);
        v.push(vector(
            format!("argon2id-{}", name.to_lowercase()),
            "argon2id",
            profile_json(name, prof),
            json!({ "input": "correct horse battery staple", "salt": hx(&[0x5Au8; 16]) }),
            json!({ "key": hx(&key) }),
        ));
    }

    // Word phrases.
    for (id, ent) in [
        ("bip39-zeros", [0u8; 32]),
        ("bip39-ff", [0xFFu8; 32]),
        ("bip39-counting", counting(32, 0).try_into().unwrap()),
    ] {
        v.push(vector(
            id,
            "bip39",
            json!({}),
            json!({ "entropy": hx(&ent) }),
            json!({ "phrase": bip39_encode(&ent) }),
        ));
    }
    let mut max10 = [0u8; 10];
    max10.copy_from_slice(&(7776u128.pow(6) - 1).to_be_bytes()[6..]);
    for (id, ent) in [
        ("eff7776-zeros", [0u8; 10]),
        ("eff7776-ones", [1u8; 10]),
        ("eff7776-counting", counting(10, 1).try_into().unwrap()),
        ("eff7776-max", max10),
    ] {
        v.push(vector(
            id,
            "eff7776",
            json!({}),
            json!({ "entropy": hx(&ent) }),
            json!({ "phrase": eff_encode(&ent).unwrap() }),
        ));
    }

    // Identity from seed, key file, sealed box, hybrid wrap.
    let recipient = DeviceIdentity::from_seed(&[0x55; 32], RECIPIENT_CTX);
    let rp = recipient.public();
    v.push(vector(
        "identity-from-seed",
        "identity_from_seed",
        json!({ "context": RECIPIENT_CTX }),
        json!({ "ikm": hx(&[0x55u8; 32]) }),
        pubs(&rp),
    ));

    let blob =
        recipient.to_encrypted_with("hunter2 passphrase", TEST_PROFILE, &[0x31; 16], &[0x32; 24]);
    v.push(vector("key-file", "key_file", json!({ "context": RECIPIENT_CTX, "salt": hx(&[0x31u8; 16]), "nonce": hx(&[0x32u8; 24]), "profile": profile_json("TEST", TEST_PROFILE) }), json!({ "ikm": hx(&[0x55u8; 32]), "passphrase": "hunter2 passphrase" }), json!({ "blob": hx(&blob) })));

    let eph_s = [0xCCu8; 32];
    let sn = [0xDDu8; 24];
    let sealed = DeviceIdentity::seal_to_with(&rp.x25519_pub, &[0x77u8; 32], &eph_s, &sn);
    assert_eq!(recipient.open_sealed(&sealed).unwrap(), vec![0x77u8; 32]);
    v.push(vector("sealed-box-share", "sealed_box", json!({ "eph_secret": hx(&eph_s), "nonce": hx(&sn), "recipient_context": RECIPIENT_CTX }), json!({ "recipient_ikm": hx(&[0x55u8; 32]), "recipient_x25519_pub": hx(&rp.x25519_pub), "plaintext": hx(&[0x77u8; 32]) }), json!({ "sealed": hx(&sealed) })));

    let (eph, wn, wm) = ([0x99u8; 32], [0xBBu8; 24], [0x1Fu8; 32]);
    for (name, pt) in [
        ("empty", vec![]),
        ("share33", vec![0x77u8; 33]),
        ("fixture64", vec![0xEEu8; 64]),
        ("long4096", vec![0x5Cu8; 4096]),
    ] {
        let wrapped = hybrid_wrap_with(&rp, &pt, &eph, &wn, &wm).unwrap();
        assert_eq!(hybrid_unwrap(&recipient, &wrapped).unwrap(), pt);
        v.push(vector(format!("hybrid_wrap-{name}"), "hybrid_wrap", json!({ "eph_secret": hx(&eph), "nonce": hx(&wn), "m": hx(&wm), "recipient_context": RECIPIENT_CTX }), json!({ "recipient_ikm": hx(&[0x55u8; 32]), "recipient": pubs(&rp), "plaintext": hx(&pt) }), json!({ "blob": hx(&wrapped) })));
    }
    // A tampered header must not open: the algorithm tag is authenticated.
    let good = hybrid_wrap_with(&rp, b"payload", &eph, &wn, &wm).unwrap();
    let mut bad_tag = good.clone();
    bad_tag[4] ^= 0x01;
    let mut bad_eph = good.clone();
    bad_eph[10] ^= 0x01;
    let mut bad_ct = good.clone();
    let last = bad_ct.len() - 1;
    bad_ct[last] ^= 1;
    for (name, blob) in [
        ("alg-tag", bad_tag),
        ("eph-pub", bad_eph),
        ("ciphertext", bad_ct),
    ] {
        v.push(vector(
            format!("hybrid_unwrap-reject-{name}"),
            "hybrid_unwrap_reject",
            json!({ "recipient_context": RECIPIENT_CTX }),
            json!({ "recipient_ikm": hx(&[0x55u8; 32]), "blob": hx(&blob) }),
            json!({ "opens": hybrid_unwrap(&recipient, &blob).is_ok() }),
        ));
    }

    // Vault and paper.
    let phrase = "abacus abdomen abdominal abide abiding ability";
    for (name, prof, slow) in [
        ("TEST", TEST_PROFILE, false),
        ("MASTER", ARGON2_MASTER, true),
    ] {
        if slow && !include_slow {
            continue;
        }
        let (public, _) = vault_recipient_with(phrase, &[0x71; 16], &[0x72; 16], prof);
        v.push(vector(format!("vault-{}", name.to_lowercase()), "vault_recipient", profile_json(name, prof), json!({ "phrase": phrase, "ring_secret": hx(&[0x71u8; 16]), "salt": hx(&[0x72u8; 16]) }), pubs(&public)));
    }
    let paper = bip39_encode(&[0x42; 32]);
    for (name, dom) in [
        ("unwrap", PaperDomain::Unwrap),
        ("recover", PaperDomain::Recover),
    ] {
        let pubk = paper_derive(&paper, dom).unwrap().public();
        v.push(vector(
            format!("paper-{name}"),
            "paper_derive",
            json!({ "domain": name }),
            json!({ "phrase": paper }),
            pubs(&pubk),
        ));
    }
    // SAS words: four BIP39 words from the first 44 bits of a transcript hash.
    for (id, hash) in [
        ("sas-zeros", [0u8; 32]),
        ("sas-ff", [0xFFu8; 32]),
        ("sas-counting", counting(32, 0x10).try_into().unwrap()),
        ("sas-chain-hash", chain_hash(b"sshub enrollment transcript")),
    ] {
        v.push(vector(
            id,
            "sas_words",
            json!({}),
            json!({ "transcript_hash": hx(&hash) }),
            json!({ "words": sas_words(&hash) }),
        ));
    }

    // Shamir and resharing. Randomness is ChaCha20Rng seeded with the recorded seed; the k - 1
    // coefficient rows are drawn as consecutive 32 byte fills.
    {
        use rand_chacha::{rand_core::SeedableRng, ChaCha20Rng};
        let share_json = |s: &Share| json!({ "index": s.index, "bytes_hex": hx(&s.bytes) });
        let secret_77 = [0x77u8; 32];
        let secret_cnt: [u8; 32] = counting(32, 0x40).try_into().unwrap();
        for (id, k, n, seed, secret) in [
            ("shamir-2of3", 2u8, 3u8, [0x11u8; 32], secret_77),
            ("shamir-3of5", 3, 5, [0x22; 32], secret_cnt),
        ] {
            let indices: Vec<u8> = (1..=n).collect();
            let shares = split_at_indices(&secret, k, &indices, &mut ChaCha20Rng::from_seed(seed));
            v.push(vector(
                id,
                "shamir_split",
                json!({ "k": k, "indices": indices, "rng_seed": hx(&seed) }),
                json!({ "secret": hx(&secret) }),
                json!({ "shares": shares.iter().map(share_json).collect::<Vec<_>>() }),
            ));
            for (suffix, subset) in [
                ("first", shares[..k as usize].to_vec()),
                ("last", shares[shares.len() - k as usize..].to_vec()),
            ] {
                assert_eq!(shamir_combine(&subset).unwrap(), secret);
                v.push(vector(
                    format!("{id}-combine-{suffix}"),
                    "shamir_combine",
                    json!({ "k": k }),
                    json!({ "shares": subset.iter().map(share_json).collect::<Vec<_>>() }),
                    json!({ "secret": hx(&secret) }),
                ));
            }
        }
        // holders 1 and 3 of a 2-of-3 sharing re-split to indices 10, 11, 12
        let old = split_at_indices(
            &secret_77,
            2,
            &[1, 2, 3],
            &mut ChaCha20Rng::from_seed([0x33; 32]),
        );
        let new_indices = [10u8, 11, 12];
        let mut contributions = Vec::new();
        for (holder, seed) in [(0usize, [0x44u8; 32]), (2, [0x55; 32])] {
            let c = reshare_contribute_with(
                &old[holder],
                &new_indices,
                2,
                &mut ChaCha20Rng::from_seed(seed),
            );
            v.push(vector(
                format!("reshare-contribute-{}", old[holder].index),
                "reshare",
                json!({ "k": 2, "new_indices": new_indices, "rng_seed": hx(&seed) }),
                json!({ "share": share_json(&old[holder]) }),
                json!({ "from": c.from,
                        "sub_shares": c.sub_shares.iter().map(|(i, b)| json!({ "index": i, "bytes_hex": hx(b) })).collect::<Vec<_>>(),
                        "commitments": c.commitments.iter().map(|c| hx(c)).collect::<Vec<_>>() }),
            ));
            contributions.push(c);
        }
        let new_shares: Vec<Share> = new_indices
            .iter()
            .map(|&i| reshare_combine(i, &contributions).unwrap())
            .collect();
        assert_eq!(shamir_combine(&new_shares[..2]).unwrap(), secret_77);
        v.push(vector("reshare-combine", "shamir_combine", json!({ "k": 2, "note": "shares assembled by resharing holders 1 and 3 to indices 10..12" }), json!({ "shares": new_shares[..2].iter().map(share_json).collect::<Vec<_>>() }), json!({ "secret": hx(&secret_77) })));
    }
    v
}

fn path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/vectors/crypto/vectors.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(vectors: Vec<Value>) -> String {
        let doc = json!({
            "meta": {
                "note": "synthetic key material (byte patterns), not real keys",
                "contract": "docs/host-sync-contract.md section 1",
                "regenerate": "SSHUB_EMIT_VECTORS=1 cargo test sync::crypto::vectors"
            },
            "vectors": vectors,
        });
        serde_json::to_string_pretty(&doc).unwrap() + "\n"
    }

    #[test]
    fn committed_vectors_match_the_implementation() {
        if std::env::var_os("SSHUB_EMIT_VECTORS").is_some() {
            std::fs::create_dir_all(path().parent().unwrap()).unwrap();
            std::fs::write(path(), document(build(true))).unwrap();
            return;
        }
        let committed: Value = serde_json::from_str(
            &std::fs::read_to_string(path())
                .expect("tests/vectors/crypto/vectors.json is committed"),
        )
        .unwrap();
        let committed = committed["vectors"].as_array().unwrap();
        for built in build(false) {
            let id = built["id"].as_str().unwrap();
            let want = committed
                .iter()
                .find(|c| c["id"] == built["id"])
                .unwrap_or_else(|| panic!("vector {id} missing from the committed file"));
            assert_eq!(&built, want, "vector {id} differs from the committed file");
        }
    }

    #[test]
    #[ignore = "runs the 1 GiB Argon2id master profile"]
    fn slow_committed_vectors_match_the_implementation() {
        let committed: Value =
            serde_json::from_str(&std::fs::read_to_string(path()).unwrap()).unwrap();
        let committed = committed["vectors"].as_array().unwrap();
        for built in build(true) {
            let id = built["id"].as_str().unwrap();
            let want = committed
                .iter()
                .find(|c| c["id"] == built["id"])
                .unwrap_or_else(|| panic!("vector {id} missing"));
            assert_eq!(&built, want, "vector {id} differs");
        }
    }

    #[test]
    fn strict_verification_rejects_what_lax_verification_accepts() {
        let built = build(false);
        for id in ["ed25519-verify-flipped", "ed25519-verify-noncanonical-s"] {
            if let Some(v) = built.iter().find(|v| v["id"] == id) {
                assert_eq!(v["outputs"]["valid"], false, "{id}");
            }
        }
        assert!(built
            .iter()
            .any(|v| v["id"] == "ed25519-verify-ok" && v["outputs"]["valid"] == true));
        for v in built
            .iter()
            .filter(|v| v["primitive"] == "hybrid_unwrap_reject")
        {
            assert_eq!(v["outputs"]["opens"], false, "{}", v["id"]);
        }
    }
}
