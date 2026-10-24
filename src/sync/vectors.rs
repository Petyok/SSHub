//! Golden vectors for the code, snapshot, email, wire and fountain modules:
//! `tests/vectors/quorum/vectors.json`, checked on every run and regenerated with
//! `SSHUB_EMIT_VECTORS=1 cargo test sync::vectors`. Inputs are self-contained (every seed,
//! nonce and profile is recorded) so the browser side can replay them and the Python oracle
//! in `tests/oracles/quorum_oracle.py` can recompute them.

use serde_json::{json, Value};

use super::codes::{crockford_encode, normalize_code};
use super::crypto::{Argon2Profile, Share};
use super::fountain::fountain_encode;
use super::wire::{binding_hash, binding_preimage, ChallengeWire, ShareResponseWire};
use super::{email, snapshot};

const TEST_PROFILE: Argon2Profile = Argon2Profile {
    mem_kib: 32 * 1024,
    iters: 1,
};

fn hx(b: &[u8]) -> String {
    hex::encode(b)
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

fn counting(n: usize, start: u8) -> Vec<u8> {
    (0..n).map(|i| start.wrapping_add(i as u8)).collect()
}

pub(crate) fn build() -> Vec<Value> {
    let mut v = Vec::new();

    for (id, bytes) in [
        ("crockford-zeros", vec![0u8; 10]),
        ("crockford-ff", vec![0xff; 10]),
        ("crockford-counting", counting(10, 1)),
        ("crockford-odd-length", counting(7, 0x80)),
    ] {
        v.push(vector(
            id,
            "crockford_encode",
            json!({}),
            json!({ "bytes": hx(&bytes) }),
            json!({ "code": crockford_encode(&bytes) }),
        ));
    }
    for (id, input) in [
        ("normalize-mixed", "k7q2-9fmx c41t-8pwd"),
        ("normalize-lookalikes", "OIl0"),
        ("normalize-rejects-u", "U"),
        ("normalize-rejects-punctuation", "K7Q2-9FM!"),
    ] {
        let r = normalize_code(input);
        v.push(vector(
            id,
            "normalize_code",
            json!({}),
            json!({ "input": input }),
            json!({ "ok": r.is_ok(), "code": r.unwrap_or_default() }),
        ));
    }

    // snapshots: the phone-side blob, and what opening it does at different times
    let share = Share {
        index: 3,
        bytes: counting(32, 0x30),
    };
    let code = "K7Q2-9FMX-C41T-8PWD";
    let (salt, nonce) = ([0x41u8; 16], [0x42u8; 24]);
    let blob = snapshot::seal_with(&share, code, 1_000_000, TEST_PROFILE, salt, nonce).unwrap();
    let params = json!({ "salt": hx(&salt), "nonce": hx(&nonce), "profile": { "profile": "TEST", "mem_kib": TEST_PROFILE.mem_kib, "iters": TEST_PROFILE.iters } });
    v.push(vector("snapshot-seal", "snapshot_seal", params, json!({ "share_index": share.index, "share_bytes": hx(&share.bytes), "code": code, "issued_at": 1_000_000 }), json!({ "blob": hx(&blob.to_bytes()) })));
    for (id, c, now) in [
        ("ok", code, 1_000_100i64),
        ("lowercase-and-spaces", "k7q2 9fmx c41t 8pwd", 1_000_100),
        ("wrong-code", "K7Q2-9FMX-C41T-8PWE", 1_000_100),
        ("expired", code, 1_000_000 + 7 * 86_400 + 1),
        ("last-second", code, 1_000_000 + 7 * 86_400),
    ] {
        let r = snapshot::snapshot_open(&blob, c, now);
        v.push(vector(format!("snapshot-open-{id}"), "snapshot_open", json!({}), json!({ "blob": hx(&blob.to_bytes()), "code": c, "now": now }), json!({ "ok": r.is_ok(), "share_index": r.as_ref().map(|s| s.index).unwrap_or(0), "share_bytes": r.map(|s| hx(&s.bytes)).unwrap_or_default() })));
    }

    // emailed code fragment
    let ecode = "ABCDE-FGHJK-MNPQR-STVWX-YZ012-3";
    let eblob = email::seal_with(
        b"fragment bytes of a share",
        ecode,
        TEST_PROFILE,
        [0x51; 16],
        [0x52; 24],
    )
    .unwrap();
    v.push(vector("email-fragment", "email_fragment_seal", json!({ "salt": hx(&[0x51u8; 16]), "nonce": hx(&[0x52u8; 24]), "profile": { "profile": "TEST", "mem_kib": TEST_PROFILE.mem_kib, "iters": TEST_PROFILE.iters } }), json!({ "fragment": hx(b"fragment bytes of a share"), "code": ecode }), json!({ "blob": hx(&eblob) })));

    // wire frames and the channel binding
    let op = br#"{"device_id":"d04ab232742bb4ab","seq":5,"lamport":5,"prev_hash":"00","timestamp":1893452400,"epoch":0,"body":{"ShareRelease":{"host_uid":"07","request_id":42,"requester":"d04ab232742bb4ab"}}}"#.to_vec();
    let ch = ChallengeWire {
        op_canonical: op,
        op_signature: [0x44; 64],
        nonce: [0xAA; 32],
        expiry: 1_893_456_000,
        eph_x25519_pub: [0x30; 32],
        request_id: 42,
    };
    let enc = ch.encode();
    assert_eq!(ChallengeWire::decode(&enc).unwrap(), ch);
    v.push(vector("wire-challenge", "wire_challenge", json!({}), json!({ "op_canonical": hx(&ch.op_canonical), "op_signature": hx(&ch.op_signature), "nonce": hx(&ch.nonce), "expiry": ch.expiry, "eph_x25519_pub": hx(&ch.eph_x25519_pub), "request_id": ch.request_id }), json!({ "frame": hx(&enc), "binding_preimage": hx(&binding_preimage(&ch.op_canonical, &ch.nonce, ch.expiry, &ch.eph_x25519_pub, ch.request_id)), "binding_hash": hx(&binding_hash(&ch.op_canonical, &ch.nonce, ch.expiry, &ch.eph_x25519_pub, ch.request_id)) })));
    let resp = ShareResponseWire {
        request_id: 42,
        ciphertext: counting(81, 7),
    };
    v.push(vector(
        "wire-share-response",
        "wire_share_response",
        json!({}),
        json!({ "request_id": resp.request_id, "ciphertext": hx(&resp.ciphertext) }),
        json!({ "frame": hx(&resp.encode()) }),
    ));

    // animated QR frames: 100 byte frames (92 byte blocks), 25% parity
    for (id, len) in [
        ("fountain-500", 500usize),
        ("fountain-84", 84),
        ("fountain-85", 85),
    ] {
        let payload = counting(len, 0x10);
        let frames = fountain_encode(&payload, 100, 25).unwrap();
        v.push(vector(
            id,
            "fountain_encode",
            json!({ "frame_bytes": 100, "parity_percent": 25 }),
            json!({ "payload": hx(&payload) }),
            json!({ "frames": frames.iter().map(|f| hx(f)).collect::<Vec<_>>() }),
        ));
    }
    v
}

fn path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/quorum/vectors.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_vectors_match_the_implementation() {
        if std::env::var_os("SSHUB_EMIT_VECTORS").is_some() {
            std::fs::create_dir_all(path().parent().unwrap()).unwrap();
            let doc = json!({ "meta": { "note": "synthetic key material, not real keys", "regenerate": "SSHUB_EMIT_VECTORS=1 cargo test sync::vectors" }, "vectors": build() });
            std::fs::write(path(), serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
            return;
        }
        let committed: Value = serde_json::from_str(
            &std::fs::read_to_string(path())
                .expect("tests/vectors/quorum/vectors.json is committed"),
        )
        .unwrap();
        let committed = committed["vectors"].as_array().unwrap();
        for built in build() {
            let id = built["id"].as_str().unwrap();
            let want = committed
                .iter()
                .find(|c| c["id"] == built["id"])
                .unwrap_or_else(|| panic!("vector {id} missing from the committed file"));
            assert_eq!(&built, want, "vector {id} differs from the committed file");
        }
    }
}
