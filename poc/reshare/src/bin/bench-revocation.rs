//! PoC B5: the price of revoking one holder, in time and log bytes, for 50, 200 and 1000
//! credentials against 2, 3, 4 and 6 holders. Machine readable JSON on stdout.
//! Run with `cargo run --release --bin bench-revocation`.

use poc_reshare::*;
use serde_json::{json, Value};
use sshub::sync::crypto::*;
use std::time::Instant;

fn wrapped_shares_json(c: &Credential, ids: &[(u8, String)], e: Enc) -> Value {
    Value::Array(c.wrapped.iter().map(|(i, blob)| json!({ "recipient": ids.iter().find(|(x, _)| x == i).unwrap().1, "alg": [1, 1], "sealed": enc(blob, e) })).collect())
}

struct Sizes {
    array: usize,
    hex: usize,
    base64: usize,
}

fn run(n_creds: usize, m: usize) -> Value {
    let holders: Vec<Holder> = (1..=m as u8).map(Holder::new).collect();
    let ids: Vec<(u8, String)> = holders
        .iter()
        .map(|h| (h.index, h.public().device_id()))
        .collect();
    let all: Vec<&Holder> = holders.iter().collect();
    let creds: Vec<Credential> = (0..n_creds)
        .map(|i| {
            create_credential(
                &all,
                format!("password-{i}-correct horse battery staple").as_bytes(),
            )
            .0
        })
        .collect();

    // the log before: one CredUpdate op per credential, written by holder 1
    let sizes_of = |f: &dyn Fn(Enc) -> usize| Sizes {
        array: f(Enc::Array),
        hex: f(Enc::Hex),
        base64: f(Enc::Base64),
    };
    let before = sizes_of(&|e| {
        creds.iter().enumerate().map(|(i, c)| {
            make_op(&holders[0].id, i as u64, 0, json!({ "CredUpdate": { "host_uid": hex::encode(c.host_uid), "ciphertext": enc(&c.ciphertext, e), "wrapped_shares": wrapped_shares_json(c, &ids, e) } })).wire_len()
        }).sum()
    });

    if m < 3 {
        return json!({ "credentials": n_creds, "holders": m, "revoke_one": "not possible: one remaining holder cannot supply k = 2 contributions, recovery needs the paper share or the phone", "log_bytes_before": { "array": before.array, "hex": before.hex, "base64": before.base64 } });
    }

    // revoke the last holder: the remaining m - 1 holders are the new set, the first two contribute
    let remaining: Vec<&Holder> = holders[..m - 1].iter().collect();
    let contributors = [remaining[0], remaining[1]];
    let new_ids: Vec<String> = remaining.iter().map(|h| h.public().device_id()).collect();

    // ---- V1: hybrid wrap per share
    let mut timing1 = Timing::default();
    let t = Instant::now();
    let r1: Vec<Reshared1> = creds
        .iter()
        .map(|c| reshare_v1(c, contributors, &remaining, &mut timing1).expect("reshare v1"))
        .collect();
    let v1_ns = t.elapsed().as_nanos();
    // every credential must still open under the new set
    for (c, r) in creds.iter().zip(&r1) {
        assert_eq!(
            unlock(&r.new_credential, remaining[0], remaining[1]).unwrap(),
            unlock(c, remaining[0], remaining[1]).unwrap()
        );
    }
    let build_v1 = |e: Enc| -> Vec<SignedOp> {
        let mut ops = Vec::new();
        for (ci, r) in r1.iter().enumerate() {
            for (k, c) in r.contribs.iter().enumerate() {
                let signer = &holders[(c.from - 1) as usize].id;
                let subs: Vec<Value> = c.subs.iter().map(|(i, b)| json!({ "recipient": ids[(*i - 1) as usize].1, "sealed": enc(b, e) })).collect();
                let commits: Vec<Value> = c.commitments.iter().map(|b| enc(b, e)).collect();
                ops.push(make_op(signer, (ci * 8 + k) as u64, 1, json!({ "ShareReshare": { "host_uid": hex::encode(c.host_uid), "epoch": 1, "new_holders": new_ids, "sub_shares": subs, "commitments": commits } })));
            }
            for (k, a) in r.adopts.iter().enumerate() {
                let signer = &holders[(a.holder - 1) as usize].id;
                ops.push(make_op(signer, (ci * 8 + 2 + k) as u64, 1, json!({ "ShareReshareConfirm": { "host_uid": hex::encode(a.host_uid), "epoch": 1, "wrapped_share": { "recipient": ids[(a.holder - 1) as usize].1, "alg": [1, 1], "sealed": enc(&a.wrapped_share, e) }, "contributors": a.contributors } })));
            }
        }
        ops
    };
    let t = Instant::now();
    let ops_v1_b64 = build_v1(Enc::Base64);
    let v1_build_ns = t.elapsed().as_nanos();
    let v1_new = sizes_of(&|e| build_v1(e).iter().map(|o| o.wire_len()).sum());

    // ---- V2: epoch envelopes
    let mut timing2 = Timing::default();
    let t = Instant::now();
    let epoch = setup_epoch(contributors, &remaining, &mut timing2).expect("epoch setup");
    let mut r2 = Vec::new();
    for c in &creds {
        r2.push(
            reshare_v2(c, &epoch, 1, contributors, &remaining, &mut timing2).expect("reshare v2"),
        );
    }
    let v2_ns = t.elapsed().as_nanos();
    // adopted shares must open and match what V1 produced mathematically: combine two V2 shares and decrypt
    for (c, (_, adopts)) in creds.iter().zip(&r2) {
        let a = open_v2_share(
            remaining[0],
            &epoch,
            1,
            &c.host_uid,
            &adopts[0].sealed_share,
        )
        .unwrap();
        let b = open_v2_share(
            remaining[1],
            &epoch,
            1,
            &c.host_uid,
            &adopts[1].sealed_share,
        )
        .unwrap();
        let dek = shamir_combine(&[a, b]).unwrap();
        assert!(
            dek_open(&dek, &c.ciphertext).is_ok(),
            "V2 reshared set must open the credential"
        );
    }
    let build_v2 = |e: Enc| -> Vec<SignedOp> {
        let mut ops = Vec::new();
        let mut seq = 0u64;
        for (from, to, blob) in &epoch.session_blobs {
            ops.push(make_op(&holders[(*from - 1) as usize].id, seq, 1, json!({ "ReshareSession": { "epoch": 1, "to": ids[(*to - 1) as usize].1, "sealed": enc(blob, e) } })));
            seq += 1;
        }
        for (h, blob) in &epoch.storage_blobs {
            ops.push(make_op(
                &holders[(*h - 1) as usize].id,
                seq,
                1,
                json!({ "EpochStorageKey": { "epoch": 1, "sealed": enc(blob, e) } }),
            ));
            seq += 1;
        }
        for (contribs, adopts) in &r2 {
            for c in contribs {
                let subs: Vec<Value> = c.subs.iter().map(|(i, b)| json!({ "recipient": ids[(*i - 1) as usize].1, "sealed": enc(b, e) })).collect();
                let commits: Vec<Value> = c.commitments.iter().map(|b| enc(b, e)).collect();
                ops.push(make_op(&holders[(c.from - 1) as usize].id, seq, 1, json!({ "ShareReshare": { "host_uid": hex::encode(c.host_uid), "epoch": 1, "new_holders": new_ids, "sub_shares": subs, "commitments": commits } })));
                seq += 1;
            }
            for a in adopts {
                ops.push(make_op(&holders[(a.holder - 1) as usize].id, seq, 1, json!({ "ShareReshareConfirm": { "host_uid": hex::encode(a.host_uid), "epoch": 1, "sealed_share": enc(&a.sealed_share, e) } })));
                seq += 1;
            }
        }
        ops
    };
    // V2 batched: one ShareReshare op per contributor and one confirm op per holder per chunk of credentials
    const CHUNK: usize = 200;
    let build_v2_batched = |e: Enc| -> Vec<SignedOp> {
        let mut ops = Vec::new();
        let mut seq = 0u64;
        for (from, to, blob) in &epoch.session_blobs {
            ops.push(make_op(&holders[(*from - 1) as usize].id, seq, 1, json!({ "ReshareSession": { "epoch": 1, "to": ids[(*to - 1) as usize].1, "sealed": enc(blob, e) } })));
            seq += 1;
        }
        for (h, blob) in &epoch.storage_blobs {
            ops.push(make_op(
                &holders[(*h - 1) as usize].id,
                seq,
                1,
                json!({ "EpochStorageKey": { "epoch": 1, "sealed": enc(blob, e) } }),
            ));
            seq += 1;
        }
        for chunk in r2.chunks(CHUNK) {
            for k in 0..2 {
                let items: Vec<Value> = chunk.iter().map(|(contribs, _)| {
                    let c = &contribs[k];
                    json!({ "host_uid": hex::encode(c.host_uid), "sub_shares": c.subs.iter().map(|(i, b)| json!({ "recipient": ids[(*i - 1) as usize].1, "sealed": enc(b, e) })).collect::<Vec<_>>(), "commitments": c.commitments.iter().map(|b| enc(b, e)).collect::<Vec<_>>() })
                }).collect();
                ops.push(make_op(&holders[(chunk[0].0[k].from - 1) as usize].id, seq, 1, json!({ "ShareReshareBatch": { "epoch": 1, "new_holders": new_ids, "items": items } })));
                seq += 1;
            }
            for k in 0..remaining.len() {
                let items: Vec<Value> = chunk.iter().map(|(_, adopts)| json!({ "host_uid": hex::encode(adopts[k].host_uid), "sealed_share": enc(&adopts[k].sealed_share, e) })).collect();
                ops.push(make_op(
                    &holders[(chunk[0].1[k].holder - 1) as usize].id,
                    seq,
                    1,
                    json!({ "ShareReshareConfirmBatch": { "epoch": 1, "items": items } }),
                ));
                seq += 1;
            }
        }
        ops
    };
    // the checkpoint state after this revocation: live credential data only (ciphertext, the live
    // envelope shares of the remaining holders, their storage key blobs); hosts, groups and
    // identities are not modelled by this bench
    let state_bytes = |e: Enc| -> usize {
        let creds: Vec<Value> = creds
            .iter()
            .zip(&r2)
            .map(|(c, (_, adopts))| {
                json!({ "host_uid": hex::encode(c.host_uid), "ciphertext": enc(&c.ciphertext, e),
                        "shares": adopts.iter().map(|a| json!({ "recipient": ids[(a.holder - 1) as usize].1, "sealed": enc(&a.sealed_share, e) })).collect::<Vec<_>>() })
            })
            .collect();
        let storage: Vec<Value> = epoch.storage_blobs.iter().map(|(h, b)| json!({ "recipient": ids[(*h - 1) as usize].1, "sealed": enc(b, e) })).collect();
        serde_json::to_vec(&json!({ "epoch": 1, "creds": creds, "storage_keys": storage })).unwrap().len()
    };
    let state_sizes = sizes_of(&|e| state_bytes(e));
    let v2b_new = sizes_of(&|e| build_v2_batched(e).iter().map(|o| o.wire_len()).sum());
    let ops_v2_b64 = build_v2(Enc::Base64);
    let v2_new = sizes_of(&|e| build_v2(e).iter().map(|o| o.wire_len()).sum());

    // ---- first sync of a fresh device: parse, hash and verify every op of the resulting log
    let before_ops: Vec<SignedOp> = creds.iter().enumerate().map(|(i, c)| make_op(&holders[0].id, i as u64, 0, json!({ "CredUpdate": { "host_uid": hex::encode(c.host_uid), "ciphertext": enc(&c.ciphertext, Enc::Base64), "wrapped_shares": wrapped_shares_json(c, &ids, Enc::Base64) } }))).collect();
    let sync_ms = |after: &[SignedOp]| -> f64 {
        let mut best = f64::MAX;
        for _ in 0..3 {
            let t = Instant::now();
            for o in before_ops.iter().chain(after) {
                assert!(o.check());
            }
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        best
    };
    let sync_v1 = sync_ms(&ops_v1_b64);
    let sync_v2 = sync_ms(&ops_v2_b64);
    let ms = |ns: u128| (ns as f64) / 1e6;
    let sz = |s: &Sizes| json!({ "array": s.array, "hex": s.hex, "base64": s.base64 });
    json!({
        "credentials": n_creds, "holders": m, "remaining": m - 1,
        "log_bytes_before": sz(&before),
        "v1_hybrid_wrap_per_share": {
            "reshare_ms_total": ms(v1_ns), "reshare_ms_busiest_holder": ms(timing1.max_ns()), "op_build_and_sign_ms": ms(v1_build_ns),
            "ops": ops_v1_b64.len(), "new_op_bytes": sz(&v1_new), "log_bytes_after_base64": before.base64 + v1_new.base64,
            "first_sync_ms_parse_hash_verify": sync_v1,
        },
        "checkpoint_state_bytes_credentials_only": sz(&state_sizes),
        "v2_batched_200_per_op": { "new_op_bytes": sz(&v2b_new), "log_bytes_after_base64": before.base64 + v2b_new.base64 },
        "v2_epoch_envelopes": {
            "reshare_ms_total": ms(v2_ns), "reshare_ms_busiest_holder": ms(timing2.max_ns()),
            "ops": ops_v2_b64.len(), "new_op_bytes": sz(&v2_new), "log_bytes_after_base64": before.base64 + v2_new.base64,
            "first_sync_ms_parse_hash_verify": sync_v2,
        },
    })
}

fn main() {
    let mut runs = Vec::new();
    for n in [50usize, 200, 1000] {
        for m in [2usize, 3, 4, 6] {
            eprintln!("credentials {n} holders {m}");
            runs.push(run(n, m));
        }
    }
    println!("{}", serde_json::to_string_pretty(&json!({ "build": if cfg!(debug_assertions) { "debug" } else { "release" }, "runs": runs })).unwrap());
}
