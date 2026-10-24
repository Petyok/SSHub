//! A2 direction (JS -> Rust): opens and verifies the blobs a JS implementation
//! produced (`blobs.json`, written by `emit-js-blobs.node.mjs` or downloaded
//! from `emit-js-blobs.html`). Prints one PASS/FAIL line per check and exits
//! non-zero on any failure.
//!
//! Usage: verify-js-blobs [path/to/blobs.json]   (default: ./blobs.json)

use anyhow::{anyhow, Context, Result};
use poc_crypto_interop::support::op::{
    decode_wire_frame, encode_share_response_frame, Challenge, Op, OpBody, ShareResponse,
};
use poc_crypto_interop::support::primitives::{
    blob_alg, hybrid_unwrap, hybrid_wrap_with_m, mlkem_encapsulate_deterministic, verify,
    DeviceIdentity, DevicePublic, ALG_CURRENT,
};
use serde_json::Value;

fn s<'a>(v: &'a Value, path: &[&str]) -> Result<&'a Value> {
    let mut cur = v;
    for k in path {
        cur = cur.get(*k).ok_or_else(|| anyhow!("missing {}", path.join(".")))?;
    }
    Ok(cur)
}

fn hexv(v: &Value, path: &[&str]) -> Result<Vec<u8>> {
    let t = s(v, path)?.as_str().ok_or_else(|| anyhow!("{} not a string", path.join(".")))?;
    Ok(hex::decode(t).with_context(|| path.join("."))?)
}

fn arr<const N: usize>(v: &Value, path: &[&str]) -> Result<[u8; N]> {
    let b = hexv(v, path)?;
    b.try_into().map_err(|b: Vec<u8>| anyhow!("{} has {} bytes, want {}", path.join("."), b.len(), N))
}

struct Report {
    fails: usize,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool) {
        if !ok {
            self.fails += 1;
        }
        println!("{} {}", if ok { "PASS" } else { "FAIL" }, name);
    }
    fn result(&mut self, name: &str, r: Result<bool>) {
        match r {
            Ok(ok) => self.check(name, ok),
            Err(e) => {
                println!("detail: {e:#}");
                self.check(name, false);
            }
        }
    }
}

fn hybrid_wrap_checks(rep: &mut Report, root: &Value) {
    rep.result("A2 hybrid_wrap: Rust opens the JS blob", (|| {
        let p = s(root, &["blobs", "hybrid_wrap", "params"])?;
        let blob = hexv(root, &["blobs", "hybrid_wrap", "output", "blob_hex"])?;
        let id = DeviceIdentity::from_parts(
            &arr::<32>(p, &["recipient_secrets_for_rust", "ed25519_seed"])?,
            &arr::<32>(p, &["recipient_secrets_for_rust", "x25519_secret"])?,
            hexv(p, &["recipient_secrets_for_rust", "mlkem_dk"])?,
        )?;
        let want = hexv(p, &["plaintext"])?;
        Ok(blob_alg(&blob)? == ALG_CURRENT && hybrid_unwrap(&id, &blob)? == want)
    })());

    rep.result("A2 hybrid_wrap: derived public keys equal the JS recipient keys", (|| {
        let p = s(root, &["blobs", "hybrid_wrap", "params"])?;
        let id = DeviceIdentity::from_parts(
            &arr::<32>(p, &["recipient_secrets_for_rust", "ed25519_seed"])?,
            &arr::<32>(p, &["recipient_secrets_for_rust", "x25519_secret"])?,
            hexv(p, &["recipient_secrets_for_rust", "mlkem_dk"])?,
        )?;
        let pb = id.public();
        Ok(pb.ed25519_pub == arr::<32>(p, &["recipient", "ed25519_pub"])?
            && pb.x25519_pub == arr::<32>(p, &["recipient", "x25519_pub"])?
            && pb.mlkem_pub == hexv(p, &["recipient", "mlkem_pub"])?)
    })());

    rep.result("A2 hybrid_wrap: Rust re-wrap with the recorded randomness is byte-identical", (|| {
        let p = s(root, &["blobs", "hybrid_wrap", "params"])?;
        let blob = hexv(root, &["blobs", "hybrid_wrap", "output", "blob_hex"])?;
        let recip = DevicePublic {
            ed25519_pub: arr::<32>(p, &["recipient", "ed25519_pub"])?,
            x25519_pub: arr::<32>(p, &["recipient", "x25519_pub"])?,
            mlkem_pub: hexv(p, &["recipient", "mlkem_pub"])?,
        };
        let (ct, ss) = mlkem_encapsulate_deterministic(&recip.mlkem_pub, &arr::<32>(p, &["mlkem_m"])?)?;
        let mine = hybrid_wrap_with_m(
            &recip,
            &hexv(p, &["plaintext"])?,
            &arr::<32>(p, &["eph_secret"])?,
            &arr::<24>(p, &["nonce"])?,
            &ct,
            &ss,
        );
        Ok(mine == blob)
    })());

    rep.result("A2 hybrid_wrap: a flipped bit in the JS blob is rejected", (|| {
        let p = s(root, &["blobs", "hybrid_wrap", "params"])?;
        let mut blob = hexv(root, &["blobs", "hybrid_wrap", "output", "blob_hex"])?;
        let id = DeviceIdentity::from_parts(
            &arr::<32>(p, &["recipient_secrets_for_rust", "ed25519_seed"])?,
            &arr::<32>(p, &["recipient_secrets_for_rust", "x25519_secret"])?,
            hexv(p, &["recipient_secrets_for_rust", "mlkem_dk"])?,
        )?;
        let last = blob.len() - 1;
        blob[last] ^= 1;
        Ok(hybrid_unwrap(&id, &blob).is_err())
    })());
}

fn share_response_checks(rep: &mut Report, root: &Value) {
    rep.result("A2 share_response: Rust decodes the SWR1 frame and opens the sealed box", (|| {
        let p = s(root, &["blobs", "share_response", "params"])?;
        let frame = hexv(root, &["blobs", "share_response", "output", "frame_hex"])?;
        let resp = decode_wire_frame(&frame)?;
        let request_id = s(p, &["request_id"])?.as_u64().ok_or_else(|| anyhow!("request_id"))?;
        // Only the X25519 secret matters for a sealed-box open; the other key
        // parts are filler borrowed from the hybrid_wrap recipient.
        let h = s(root, &["blobs", "hybrid_wrap", "params", "recipient_secrets_for_rust"])?;
        let id = DeviceIdentity::from_parts(
            &arr::<32>(h, &["ed25519_seed"])?,
            &arr::<32>(p, &["challenge_eph_secret_for_rust"])?,
            hexv(h, &["mlkem_dk"])?,
        )?;
        Ok(resp.request_id == request_id && id.open_sealed(&resp.ciphertext)? == hexv(p, &["plaintext"])?)
    })());

    rep.result("A2 share_response: Rust re-seal and re-frame is byte-identical", (|| {
        let p = s(root, &["blobs", "share_response", "params"])?;
        let frame = hexv(root, &["blobs", "share_response", "output", "frame_hex"])?;
        let ct = DeviceIdentity::seal_to_seeded(
            &arr::<32>(p, &["challenge_eph_pub"])?,
            &hexv(p, &["plaintext"])?,
            &arr::<32>(p, &["sealer_eph_secret"])?,
            &arr::<24>(p, &["nonce"])?,
        );
        let request_id = s(p, &["request_id"])?.as_u64().ok_or_else(|| anyhow!("request_id"))?;
        let mine = encode_share_response_frame(&ShareResponse { request_id, ciphertext: ct })?;
        Ok(mine == frame)
    })());
}

fn op_from_json(v: &Value) -> Result<Op> {
    let b = s(v, &["body", "ShareRelease"])?;
    Ok(Op {
        device_id: s(v, &["device_id"])?.as_str().ok_or_else(|| anyhow!("device_id"))?.to_string(),
        seq: s(v, &["seq"])?.as_u64().ok_or_else(|| anyhow!("seq"))?,
        lamport: s(v, &["lamport"])?.as_u64().ok_or_else(|| anyhow!("lamport"))?,
        prev_hash: arr::<32>(v, &["prev_hash"])?,
        timestamp: s(v, &["timestamp"])?.as_i64().ok_or_else(|| anyhow!("timestamp"))?,
        epoch: s(v, &["epoch"])?.as_u64().ok_or_else(|| anyhow!("epoch"))?,
        body: OpBody::ShareRelease {
            host_id: s(b, &["host_id"])?.as_i64().ok_or_else(|| anyhow!("host_id"))?,
            request_id: s(b, &["request_id"])?.as_u64().ok_or_else(|| anyhow!("request_id"))?,
            requester: s(b, &["requester"])?.as_str().ok_or_else(|| anyhow!("requester"))?.to_string(),
        },
        signature: arr::<64>(v, &["signature"])?,
    })
}

fn approval_checks(rep: &mut Report, root: &Value) {
    let load = || -> Result<(Challenge, &Value)> {
        let p = s(root, &["blobs", "signed_approval", "params"])?;
        let op = op_from_json(s(p, &["op"])?)?;
        let ch = Challenge {
            op,
            nonce: arr::<32>(p, &["nonce"])?,
            expiry: s(p, &["expiry"])?.as_i64().ok_or_else(|| anyhow!("expiry"))?,
            eph_x25519_pub: arr::<32>(p, &["eph_x25519_pub"])?,
            request_id: s(p, &["request_id"])?.as_u64().ok_or_else(|| anyhow!("request_id"))?,
        };
        Ok((ch, p))
    };
    let out = |k: &str| hexv(root, &["blobs", "signed_approval", "output", k]);

    rep.result("A2 approval: Rust verifies the JS requester signature on the op", (|| {
        let (ch, p) = load()?;
        Ok(ch.op.verify(&arr::<32>(p, &["requester_pub"])?))
    })());
    rep.result("A2 approval: op hash from Rust equals the JS op hash", (|| {
        let (ch, _) = load()?;
        Ok(ch.op.hash().to_vec() == out("op_hash_hex")?)
    })());
    rep.result("A2 approval: binding preimage and binding hash are byte-identical", (|| {
        let (ch, _) = load()?;
        Ok(ch.preimage_bytes() == out("preimage_hex")? && ch.binding_hash().to_vec() == out("binding_hash_hex")?)
    })());
    rep.result("A2 approval: Rust verifies the JS holder signature over the preimage", (|| {
        let (ch, p) = load()?;
        let sig: [u8; 64] = out("sig_hex")?.try_into().map_err(|_| anyhow!("sig length"))?;
        Ok(verify(&arr::<32>(p, &["holder_pub"])?, &ch.preimage_bytes(), &sig))
    })());
    rep.result("A2 approval: signature does not verify for a different requester id", (|| {
        let (mut ch, p) = load()?;
        if let OpBody::ShareRelease { requester, .. } = &mut ch.op.body {
            requester.push('0');
        }
        let sig: [u8; 64] = out("sig_hex")?.try_into().map_err(|_| anyhow!("sig length"))?;
        Ok(!verify(&arr::<32>(p, &["holder_pub"])?, &ch.preimage_bytes(), &sig))
    })());
}

fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "blobs.json".to_string());
    let text = std::fs::read_to_string(&path).with_context(|| format!("read {path}"))?;
    let root: Value = serde_json::from_str(&text)?;
    let mut rep = Report { fails: 0 };
    hybrid_wrap_checks(&mut rep, &root);
    share_response_checks(&mut rep, &root);
    approval_checks(&mut rep, &root);
    println!("{}", if rep.fails == 0 { "RESULT PASS" } else { "RESULT FAIL" });
    if rep.fails > 0 {
        std::process::exit(1);
    }
    Ok(())
}
