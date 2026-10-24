//! Emits `vectors.json` (RECON §5 schema): every primitive from PoC spec A1/A2
//! with recorded seeds and fixed key material. Deterministic: rerunning this
//! binary produces identical output apart from `meta.generated`.

use anyhow::Result;
use ed25519_dalek::{Signer, SigningKey};
use poc_crypto_interop::support::op::{
    device_id_of, encode_share_response_frame, Challenge, CredMode, Op, OpBody, ShareResponse,
    ThirdFactor, PROTO_VERSION,
};
use poc_crypto_interop::support::primitives::{
    argon2_derive, blob_alg, chain_hash, dek_open, dek_seal_with_nonce, hybrid_unwrap,
    mlkem_decapsulate, mlkem_encapsulate_deterministic, mlkem_generate_deterministic, verify,
    ARGON2_EMAIL, ARGON2_MASTER, ARGON2_SNAPSHOT, ARGON2_TEST,
};
use poc_crypto_interop::support::shamir::{
    reshare_combine, reshare_contribute_with_rng, shamir_combine, shamir_split_seeded, Share,
};
use poc_crypto_interop::support::words::{bip39_decode, bip39_encode, eff_encode};
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha20Rng;
use serde_json::{json, Value};

// ---------------------------------------------------------------- fixtures
// Shared with the JS side (test/fixtures.mjs) - do not change unilaterally.

const REQ_ED_SEED: [u8; 32] = [0x11; 32];
const REQ_X_SEC: [u8; 32] = [0x22; 32];
const HOLDER_ED_SEED: [u8; 32] = [0x33; 32];
const RECIP_ED_SEED: [u8; 32] = [0x44; 32];
const RECIP_X_SEC: [u8; 32] = [0x55; 32];
const RECIP_MLKEM_D: [u8; 32] = [0x66; 32];
const RECIP_MLKEM_Z: [u8; 32] = [0x77; 32];
const CHAL_EPH_X_SEC: [u8; 32] = [0x88; 32];
const CHAL_NONCE: [u8; 32] = [0xAA; 32];
const CHAL_EXPIRY: i64 = 1_893_456_000;
const REQUEST_ID: u64 = 42;
const WRAP_EPH_SEC: [u8; 32] = [0x99; 32];
const WRAP_NONCE: [u8; 24] = [0xBB; 24];
const SHARE_EPH_SEC: [u8; 32] = [0xCC; 32];
const SHARE_NONCE: [u8; 24] = [0xDD; 24];
const WRAP_PT: [u8; 64] = [0xEE; 64];
const SHARE_PT: [u8; 32] = [0x77; 32];
const OP_SEQ: u64 = 1;
const OP_LAMPORT: u64 = 1;
const OP_TS: i64 = 1_893_452_400;
const OP_EPOCH: u64 = 0;
const OP_HOST_ID: i64 = 7;

fn counting(n: usize, start: u8) -> Vec<u8> {
    (0..n).map(|i| start.wrapping_add(i as u8)).collect()
}

fn hex(b: &[u8]) -> Value {
    json!(hex::encode(b))
}

struct V {
    id: String,
    primitive: &'static str,
    params: Value,
    inputs: Value,
    outputs: Value,
}

fn v(id: impl Into<String>, primitive: &'static str, params: Value, inputs: Value, outputs: Value) -> V {
    V { id: id.into(), primitive, params, inputs, outputs }
}

fn op_input(op: &Op) -> Value {
    json!({
        "device_id": op.device_id,
        "seq": op.seq,
        "lamport": op.lamport,
        "prev_hash": hex::encode(op.prev_hash),
        "timestamp": op.timestamp,
        "epoch": op.epoch,
        "body": serde_json::to_value(&op.body).expect("body json"),
        "signature": hex::encode(op.signature),
    })
}

/// 7776^6 as a u128 (const loop).
const fn pow6(base: u128) -> u128 {
    let mut acc: u128 = 1;
    let mut i = 0;
    while i < 6 {
        acc *= base;
        i += 1;
    }
    acc
}

/// RFC3339 UTC timestamp from the system clock (no chrono dep).
fn rfc3339_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let s = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        s / 3600,
        (s % 3600) / 60,
        s % 60
    )
}

fn requester_id() -> String {
    device_id_of(&req_pub())
}

fn req_pub() -> [u8; 32] {
    SigningKey::from_bytes(&REQ_ED_SEED).verifying_key().to_bytes()
}

fn x25519_pub(sec: &[u8; 32]) -> [u8; 32] {
    use x25519_dalek::{PublicKey as XPublic, StaticSecret as XSecret};
    XPublic::from(&XSecret::from(*sec)).to_bytes()
}

fn x25519_shared(a_sec: &[u8; 32], b_pub: &[u8; 32]) -> [u8; 32] {
    use x25519_dalek::{PublicKey as XPublic, StaticSecret as XSecret};
    XSecret::from(*a_sec).diffie_hellman(&XPublic::from(*b_pub)).to_bytes()
}

/// The base ShareRelease op fixture: genesis header (prev_hash = 00*32, Q6).
fn base_release_op() -> Op {
    Op {
        device_id: device_id_of(&req_pub()),
        seq: OP_SEQ,
        lamport: OP_LAMPORT,
        prev_hash: [0u8; 32],
        timestamp: OP_TS,
        epoch: OP_EPOCH,
        body: OpBody::ShareRelease {
            host_id: OP_HOST_ID,
            request_id: REQUEST_ID,
            requester: requester_id(),
        },
        signature: [0u8; 64],
    }
}

fn main() -> Result<()> {
    let mut vectors: Vec<V> = Vec::new();

    // ---------------------------------------------------------- chain_hash
    for (id, bytes) in [
        ("chain_hash-empty", Vec::new()),
        ("chain_hash-sshub", b"sshub".to_vec()),
    ] {
        vectors.push(v(
            id,
            "chain_hash",
            json!({}),
            json!({ "bytes": hex::encode(&bytes) }),
            json!({ "hash": hex::encode(chain_hash(&bytes)) }),
        ));
    }

    // ---------------------------------------------------------- derive_key
    {
        let ikm = counting(32, 0x20);
        for ctx in [
            "sshub/sync/wrap/v1",
            "sshub/sync/seed/x25519/v1",
            "sshub/sync/vault/v1",
            "sshub/sync/paper/unwrap/v1",
            "sshub/sync/paper/recover/v1",
        ] {
            let key = blake3::derive_key(ctx, &ikm);
            vectors.push(v(
                format!("derive_key-{}", ctx.rsplit('/').nth(1).unwrap_or(ctx)),
                "derive_key",
                json!({ "context": ctx }),
                json!({ "ikm": hex::encode(&ikm) }),
                json!({ "key": hex::encode(key) }),
            ));
        }
    }

    // ---------------------------------------------------------- ed25519_sign
    {
        let sk = SigningKey::from_bytes(&REQ_ED_SEED);
        let msg = b"sshub/sync/test message".to_vec();
        let sig = sk.sign(&msg).to_bytes();
        let pub_bytes = sk.verifying_key().to_bytes();
        assert!(verify(&pub_bytes, &msg, &sig));
        vectors.push(v(
            "ed25519_sign-1",
            "ed25519_sign",
            json!({ "msg_encoding": "utf8" }),
            json!({ "seed": hex::encode(REQ_ED_SEED), "msg": hex::encode(&msg) }),
            json!({ "pub": hex::encode(pub_bytes), "sig": hex::encode(sig), "verify_ok": true }),
        ));
    }

    // ---------------------------------------------------------- x25519_dh
    {
        let a_sec = REQ_X_SEC;
        let b_pub = x25519_pub(&CHAL_EPH_X_SEC);
        let shared = {
            use x25519_dalek::{PublicKey as XPublic, StaticSecret as XSecret};
            XSecret::from(a_sec).diffie_hellman(&XPublic::from(b_pub)).to_bytes()
        };
        vectors.push(v(
            "x25519_dh-1",
            "x25519_dh",
            json!({ "b_sec": hex::encode(CHAL_EPH_X_SEC) }),
            json!({ "a_sec": hex::encode(a_sec), "b_pub": hex::encode(b_pub) }),
            json!({ "a_pub": hex::encode(x25519_pub(&a_sec)), "shared": hex::encode(shared) }),
        ));
    }

    // ---------------------------------------------------------- mlkem
    {
        let (ek, dk) = mlkem_generate_deterministic(&RECIP_MLKEM_D, &RECIP_MLKEM_Z);
        assert_eq!(ek.len(), 1184);
        assert_eq!(dk.len(), 2400);
        let m = counting(32, 0x00);
        let mut m32 = [0u8; 32];
        m32.copy_from_slice(&m);
        let (ct, ss) = mlkem_encapsulate_deterministic(&ek, &m32)?;
        let ss_back = mlkem_decapsulate(&dk, &ct)?;
        assert_eq!(ss, ss_back);

        vectors.push(v(
            "mlkem_encaps-1",
            "mlkem_encaps",
            json!({ "d": hex::encode(RECIP_MLKEM_D), "z": hex::encode(RECIP_MLKEM_Z), "m": hex::encode(&m) }),
            json!({ "ek": hex::encode(&ek) }),
            json!({ "ct": hex::encode(&ct), "ss": hex::encode(ss) }),
        ));
        vectors.push(v(
            "mlkem_decaps-1",
            "mlkem_decaps",
            json!({ "d": hex::encode(RECIP_MLKEM_D), "z": hex::encode(RECIP_MLKEM_Z) }),
            json!({ "dk": hex::encode(&dk), "ct": hex::encode(&ct) }),
            json!({ "ss": hex::encode(ss) }),
        ));
    }

    // ---------------------------------------------------------- dek_seal
    {
        let dek = [0xD3u8; 32];
        for (id, pt) in [
            ("dek_seal-empty", Vec::new()),
            ("dek_seal-1", vec![0x77u8]),
            ("dek_seal-40", vec![0x77u8; 40]),
            ("dek_seal-1024", vec![0x77u8; 1024]),
        ] {
            let ct = dek_seal_with_nonce(&dek, &pt, &WRAP_NONCE);
            assert_eq!(dek_open(&dek, &ct).unwrap(), pt);
            vectors.push(v(
                id,
                "dek_seal",
                json!({ "nonce": hex::encode(WRAP_NONCE) }),
                json!({ "dek": hex::encode(dek), "plaintext": hex::encode(&pt) }),
                json!({ "ct": hex::encode(&ct) }),
            ));
        }
    }

    // ---------------------------------------------------------- argon2id
    {
        let input = "correct horse battery staple";
        let salt16: [u8; 16] = [0x5A; 16];
        for (name, p) in [
            ("TEST", ARGON2_TEST),
            ("SNAPSHOT", ARGON2_SNAPSHOT),
            ("EMAIL", ARGON2_EMAIL),
            ("MASTER", ARGON2_MASTER),
        ] {
            let key = argon2_derive(input, &salt16, p);
            vectors.push(v(
                format!("argon2id-{}", name.to_lowercase()),
                "argon2id",
                json!({ "profile": name, "mem_kib": p.mem_kib, "iters": p.iters, "lanes": 1, "out_len": 32, "version": "0x13" }),
                json!({ "input": input, "salt": hex::encode(salt16) }),
                json!({ "key": hex::encode(key) }),
            ));
        }
    }

    // ---------------------------------------------------------- bip39
    {
        let ent_count: Vec<u8> = counting(32, 0x00);
        let mut ent_counting = [0u8; 32];
        ent_counting.copy_from_slice(&ent_count);
        for (id, ent) in [
            ("bip39-zeros", [0u8; 32]),
            ("bip39-ff", [0xFFu8; 32]),
            ("bip39-counting", ent_counting),
        ] {
            let phrase = bip39_encode(&ent);
            assert_eq!(bip39_decode(&phrase)?, ent);
            vectors.push(v(
                id,
                "bip39",
                json!({}),
                json!({ "entropy": hex::encode(ent) }),
                json!({ "phrase": phrase, "decode_ok": true }),
            ));
        }
    }

    // ---------------------------------------------------------- eff7776
    {
        let max = pow6(7776) - 1;
        let max_bytes = max.to_be_bytes();
        let mut max10 = [0u8; 10];
        max10.copy_from_slice(&max_bytes[6..]);
        let mut count10 = [0u8; 10];
        count10.copy_from_slice(&counting(10, 0x01));
        for (id, ent) in [
            ("eff7776-zeros", [0u8; 10]),
            ("eff7776-ones", [0x01u8; 10]),
            ("eff7776-counting", count10),
            ("eff7776-max", max10),
        ] {
            let phrase = eff_encode(&ent)?;
            vectors.push(v(
                id,
                "eff7776",
                json!({}),
                json!({ "entropy": hex::encode(ent) }),
                json!({ "phrase": phrase }),
            ));
        }
    }

    // ---------------------------------------------------------- shamir + reshare
    {
        let secret_77 = [0x77u8; 32];
        let mut secret_cnt = [0u8; 32];
        secret_cnt.copy_from_slice(&counting(32, 0x40));
        let share_json = |s: &Share| json!({ "index": s.index, "bytes_hex": hex::encode(&s.bytes) });
        for (id, k, n, seed, secret) in [
            ("shamir-2of3", 2u8, 3u8, [0x11u8; 32], secret_77),
            ("shamir-3of5", 3u8, 5u8, [0x22u8; 32], secret_cnt),
        ] {
            let mut rng = ChaCha20Rng::from_seed(seed);
            let shares = shamir_split_seeded(&secret, k, n, &mut rng);
            vectors.push(v(
                id,
                "shamir_split",
                json!({ "k": k, "n": n, "rng_seed": hex::encode(seed) }),
                json!({ "secret": hex::encode(secret) }),
                json!({ "shares": shares.iter().map(share_json).collect::<Vec<_>>() }),
            ));
            let first: Vec<Share> = shares[..k as usize].to_vec();
            let last: Vec<Share> = shares[shares.len() - k as usize..].to_vec();
            for (suffix, subset) in [("first", first), ("last", last)] {
                assert_eq!(shamir_combine(&subset)?, secret);
                vectors.push(v(
                    format!("{id}-combine-{suffix}"),
                    "shamir_combine",
                    json!({ "k": k }),
                    json!({ "shares": subset.iter().map(share_json).collect::<Vec<_>>() }),
                    json!({ "secret": hex::encode(secret) }),
                ));
            }
        }

        // reshare: holders 1 and 3 of a 2-of-3 sharing re-split to indices 10..12.
        let mut rng = ChaCha20Rng::from_seed([0x33u8; 32]);
        let shares = shamir_split_seeded(&secret_77, 2, 3, &mut rng);
        let new_indices = [10u8, 11, 12];
        let mut contributions = Vec::new();
        for (holder, seed) in [(0usize, [0x44u8; 32]), (2usize, [0x55u8; 32])] {
            let mut crng = ChaCha20Rng::from_seed(seed);
            let c = reshare_contribute_with_rng(&shares[holder], &new_indices, 2, &mut crng);
            vectors.push(v(
                format!("reshare-contribute-{}", shares[holder].index),
                "reshare",
                json!({ "k": 2, "new_indices": new_indices, "rng_seed": hex::encode(seed) }),
                json!({ "share": share_json(&shares[holder]) }),
                json!({ "from": c.from, "sub_shares": c.sub_shares.iter()
                    .map(|(i, b)| json!({ "index": i, "bytes_hex": hex::encode(b) })).collect::<Vec<_>>() }),
            ));
            contributions.push(c);
        }
        let new_shares: Vec<Share> = new_indices
            .iter()
            .map(|&i| reshare_combine(i, &contributions))
            .collect::<Result<_>>()?;
        assert_eq!(shamir_combine(&new_shares[..2])?, secret_77);
        vectors.push(v(
            "reshare-combine",
            "shamir_combine",
            json!({ "k": 2, "note": "shares assembled by resharing holders 1 and 3 to indices 10..12" }),
            json!({ "shares": new_shares[..2].iter().map(share_json).collect::<Vec<_>>() }),
            json!({ "secret": hex::encode(secret_77) }),
        ));
    }

    // ---------------------------------------------------------- ops (A5 edge cases)
    let req_sk = SigningKey::from_bytes(&REQ_ED_SEED);
    let rid = requester_id();
    let mk = |seq: u64, lamport: u64, timestamp: i64, body: OpBody| -> Op {
        let mut op = Op {
            device_id: rid.clone(),
            seq,
            lamport,
            prev_hash: [0u8; 32],
            timestamp,
            epoch: 0,
            body,
            signature: [0u8; 64],
        };
        op.signature = req_sk.sign(&op.canonical_bytes()).to_bytes();
        op
    };
    let ops: Vec<(&str, Op)> = vec![
        (
            "op-host-upsert-cyrillic-emoji",
            mk(1, 1, OP_TS, OpBody::HostUpsert {
                host_id: 7,
                name: "\u{425}\u{43e}\u{441}\u{442} \u{26a1} \u{1f680}".to_string(),
                address: "h.example".into(),
                port: 22,
                fields_json: "{\"k\":\"v\"}".into(),
            }),
        ),
        (
            "op-host-upsert-control-chars",
            mk(2, 2, OP_TS, OpBody::HostUpsert {
                host_id: -1,
                name: "a\u{1}b\"c\\d\ne\u{7f}\u{2028}".into(),
                address: "".into(),
                port: 65535,
                fields_json: "".into(),
            }),
        ),
        (
            "op-cred-update-empty",
            mk(3, 3, OP_TS, OpBody::CredUpdate { host_id: 7, ciphertext: vec![], wrapped_shares: vec![] }),
        ),
        (
            "op-cred-update-nonempty",
            mk(4, 4, OP_TS, OpBody::CredUpdate {
                host_id: 7,
                ciphertext: vec![1, 2, 3, 255, 0],
                wrapped_shares: vec![
                    poc_crypto_interop::support::op::WrappedShare { recipient: "aa11".into(), sealed: vec![] },
                    poc_crypto_interop::support::op::WrappedShare { recipient: "bb22".into(), sealed: vec![0xFF; 3] },
                ],
            }),
        ),
        ("op-share-release", mk(5, 5, OP_TS, OpBody::ShareRelease { host_id: OP_HOST_ID, request_id: REQUEST_ID, requester: rid.clone() })),
        (
            "op-reenroll-cancel-none",
            mk(6, 6, OP_TS, OpBody::PhoneReenrollCancel { reenroll_hash: [0xAB; 32], by_phone: true, third_factor: None }),
        ),
        (
            "op-reenroll-cancel-paper",
            mk(7, 7, OP_TS, OpBody::PhoneReenrollCancel { reenroll_hash: [0; 32], by_phone: false, third_factor: Some(ThirdFactor::Paper) }),
        ),
        (
            "op-reenroll-cancel-hwkey",
            mk(8, 8, OP_TS, OpBody::PhoneReenrollCancel {
                reenroll_hash: [0x01; 32],
                by_phone: false,
                third_factor: Some(ThirdFactor::HardwareKey("abc".into())),
            }),
        ),
        (
            "op-head-attest-u64max",
            mk(u64::MAX, u64::MAX, i64::MIN, OpBody::HeadAttest { device_id: rid.clone(), seq: u64::MAX, chain_hash: [0xFF; 32] }),
        ),
        (
            "op-mode-change-none",
            mk(9, 9, i64::MAX, OpBody::ModeChange { to: CredMode::Quorum, migrated_on: rid.clone(), cosigner: None }),
        ),
        (
            "op-mode-change-some",
            mk(10, 10, 0, OpBody::ModeChange { to: CredMode::Vault, migrated_on: rid.clone(), cosigner: Some("cc33".into()) }),
        ),
        (
            "op-reshare-confirm",
            mk(11, 11, OP_TS, OpBody::ShareReshareConfirm { reshare_hash: [0x5A; 32], by: rid.clone() }),
        ),
    ];
    for (id, op) in &ops {
        let pubk = req_pub();
        assert!(op.verify(&pubk));
        vectors.push(v(
            format!("{id}-canonical"),
            "op_canonical",
            json!({}),
            json!({ "op": op_input(op) }),
            json!({ "canonical_hex": hex::encode(op.canonical_bytes()), "canonical_text": String::from_utf8_lossy(&op.canonical_bytes()) }),
        ));
        vectors.push(v(
            format!("{id}-hash"),
            "op_hash",
            json!({}),
            json!({ "op": op_input(op) }),
            json!({ "hash": hex::encode(op.hash()) }),
        ));
        vectors.push(v(
            format!("{id}-verify"),
            "op_verify",
            json!({}),
            json!({ "op": op_input(op), "pub": hex::encode(pubk) }),
            json!({ "valid": true }),
        ));
    }
    {
        // A tampered op keeps the old signature and must not verify.
        let mut bad = ops[4].1.clone();
        bad.seq += 1;
        assert!(!bad.verify(&req_pub()));
        vectors.push(v(
            "op-share-release-tampered-verify",
            "op_verify",
            json!({}),
            json!({ "op": op_input(&bad), "pub": hex::encode(req_pub()) }),
            json!({ "valid": false }),
        ));
    }

    // ---------------------------------------------------------- binding hash + approval
    let chal_eph_pub = x25519_pub(&CHAL_EPH_X_SEC);
    let holder_sk = SigningKey::from_bytes(&HOLDER_ED_SEED);
    for (id, idx) in [("binding-share-release", 4usize), ("binding-host-upsert", 0), ("binding-head-attest-u64max", 8)] {
        let ch = Challenge {
            op: ops[idx].1.clone(),
            nonce: CHAL_NONCE,
            expiry: CHAL_EXPIRY,
            eph_x25519_pub: chal_eph_pub,
            request_id: REQUEST_ID,
        };
        vectors.push(v(
            id,
            "binding_hash",
            json!({ "proto_version": PROTO_VERSION }),
            json!({ "op": op_input(&ch.op), "nonce": hex::encode(ch.nonce), "expiry": ch.expiry,
                    "eph": hex::encode(ch.eph_x25519_pub), "request_id": ch.request_id }),
            json!({ "binding_hash": hex::encode(ch.binding_hash()), "preimage": hex::encode(ch.preimage_bytes()) }),
        ));
        let sig = holder_sk.sign(&ch.preimage_bytes()).to_bytes();
        vectors.push(v(
            format!("{id}-approval"),
            "signed_approval",
            json!({ "sig_over": "preimage", "holder_seed": hex::encode(HOLDER_ED_SEED) }),
            json!({ "op": op_input(&ch.op), "nonce": hex::encode(ch.nonce), "expiry": ch.expiry,
                    "eph": hex::encode(ch.eph_x25519_pub), "request_id": ch.request_id,
                    "holder_pub": hex::encode(holder_sk.verifying_key().to_bytes()) }),
            json!({ "preimage": hex::encode(ch.preimage_bytes()), "sig": hex::encode(sig) }),
        ));
    }

    // ---------------------------------------------------------- hybrid wrap
    {
        let (ek, dk) = mlkem_generate_deterministic(&RECIP_MLKEM_D, &RECIP_MLKEM_Z);
        let recip = poc_crypto_interop::support::primitives::DevicePublic {
            ed25519_pub: SigningKey::from_bytes(&RECIP_ED_SEED).verifying_key().to_bytes(),
            x25519_pub: x25519_pub(&RECIP_X_SEC),
            mlkem_pub: ek,
        };
        let id = poc_crypto_interop::support::primitives::DeviceIdentity::from_parts(&RECIP_ED_SEED, &RECIP_X_SEC, dk)?;
        let m = [0x1Fu8; 32];
        let (kem_ct, kem_ss) = mlkem_encapsulate_deterministic(&recip.mlkem_pub, &m)?;
        for (name, pt) in [("empty", Vec::new()), ("share32", SHARE_PT.to_vec()), ("fixture64", WRAP_PT.to_vec()), ("long4096", vec![0x5Cu8; 4096])] {
            let blob = poc_crypto_interop::support::primitives::hybrid_wrap_with_m(&recip, &pt, &WRAP_EPH_SEC, &WRAP_NONCE, &kem_ct, &kem_ss);
            assert_eq!(hybrid_unwrap(&id, &blob)?, pt);
            assert_eq!(blob_alg(&blob)?, poc_crypto_interop::support::primitives::ALG_CURRENT);
            vectors.push(v(
                format!("hybrid_wrap-{name}"),
                "hybrid_wrap",
                json!({ "eph_secret": hex::encode(WRAP_EPH_SEC), "nonce": hex::encode(WRAP_NONCE), "m": hex::encode(m) }),
                json!({ "recipient": { "ed25519_pub": hex::encode(recip.ed25519_pub), "x25519_pub": hex::encode(recip.x25519_pub),
                                       "mlkem_pub": hex::encode(&recip.mlkem_pub) },
                        "plaintext": hex::encode(&pt) }),
                json!({ "blob": hex::encode(&blob) }),
            ));
        }
    }

    // ---------------------------------------------------------- sealed box + share response
    {
        let ct = poc_crypto_interop::support::primitives::DeviceIdentity::seal_to_seeded(&chal_eph_pub, &SHARE_PT, &SHARE_EPH_SEC, &SHARE_NONCE);
        vectors.push(v(
            "sealed_box-share",
            "sealed_box",
            json!({ "sealer_eph_secret": hex::encode(SHARE_EPH_SEC), "nonce": hex::encode(SHARE_NONCE) }),
            json!({ "recipient_x25519_pub": hex::encode(chal_eph_pub), "plaintext": hex::encode(SHARE_PT),
                    "recipient_x25519_secret": hex::encode(CHAL_EPH_X_SEC) }),
            json!({ "sealed": hex::encode(&ct) }),
        ));
        let frame = encode_share_response_frame(&ShareResponse { request_id: REQUEST_ID, ciphertext: ct })?;
        vectors.push(v(
            "share_response-1",
            "share_response",
            json!({}),
            json!({ "eph_x25519_pub": hex::encode(chal_eph_pub), "plaintext": hex::encode(SHARE_PT) }),
            json!({ "frame": hex::encode(&frame) }),
        ));
    }

    // ---------------------------------------------------------- write
    let rustc = std::process::Command::new("rustc")
        .arg("-V")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let doc = json!({
        "meta": {
            "generated": rfc3339_now(),
            "proto_version": PROTO_VERSION,
            "domain_tag_challenge": "sshub/sync/challenge/v1",
            "wrap_kdf_context": "sshub/sync/wrap/v1",
            "note": "all key material is synthetic fixture data (byte patterns), not real keys",
            "rust": { "rustc": rustc, "crate": env!("CARGO_PKG_NAME") },
        },
        "vectors": vectors.iter().map(|x| json!({
            "id": x.id, "primitive": x.primitive, "params": x.params, "inputs": x.inputs, "outputs": x.outputs,
        })).collect::<Vec<_>>(),
    });
    let out = std::env::args().nth(1).unwrap_or_else(|| "vectors.json".to_string());
    std::fs::write(&out, serde_json::to_string_pretty(&doc)? + "\n")?;
    println!("wrote {out} with {} vectors", vectors.len());
    Ok(())
}
