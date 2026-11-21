//! `assets/companion/test/fixtures/notices.json`: one Red notice (the Red case is opened at its own `open_at`), one trust request (kind 9) and one trust extension
//! (kind 10) as the terminal's encoders write them, plus what a fresh phone (secret 42 x 32, the phone of
//! `challenges.json`) shows for each. The Android real-core tests read the file next to `challenges.json`.
//!
//! The file is generated here and this test fails when the committed one differs:
//! `SSHUB_EMIT_PHONE_FIXTURES=1 cargo test -p sshub-phone-core --test notices_fixture` rewrites it.
//!
//! Flow for the extension case: open the `trust-request` frame on a fresh phone at `now`, approve it at `now` with the
//! delegate id typed (`trust-request.approve.typed_id`), then open the `trust-extension` frame at `now + open_after_secs`.
//! The certificate id inside that frame is the hash of the `CertIssue` op the phone signs in that flow; on a fresh phone
//! (`fresh_phone_problems`) the same frame is refused because that phone issued nothing.

use serde_json::{json, Value};
use sshub_phone_core::ffi::PhoneCore;
use sshub_phone_core::views::{Decision, Opened, Row};
use sshub_sync_proto::oplog::CertScope;
use sshub_sync_proto::wire::{CertExtendRequestWire, CertRequestWire, RecoveryNoticeWire};

const FIX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/companion/test/fixtures/"
);
const DAY: i64 = 86_400;
const DELEGATE: &str = "cccccccccccccccc";

fn load(name: &str) -> Value {
    let text = std::fs::read_to_string(format!("{FIX}{name}")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn fresh() -> std::sync::Arc<PhoneCore> {
    let c = PhoneCore::new(None);
    c.unlock(vec![0x42; 32]).unwrap();
    c
}

fn rows(rows: &[Row]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| json!({ "label": r.label, "value": r.value, "danger": r.danger }))
            .collect(),
    )
}

fn build() -> Value {
    let ch = load("challenges.json");
    let now = ch["now"].as_i64().unwrap();

    // Red notice: the first Red op of ops.vectors.json (signed by the vectors' terminal key, not in a fresh phone's
    // ring), as a kind 3 frame, read 100 s after it was signed.
    let page = load("parity-page.json");
    let vectors = load("ops.vectors.json");
    let w = &page["red_window"][0];
    let id = w["id"].as_str().unwrap();
    let ts = w["timestamp"].as_i64().unwrap();
    let v = vectors["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == id)
        .unwrap();
    let red_frame = RecoveryNoticeWire {
        op_canonical: hex::decode(v["canonical_hex"].as_str().unwrap()).unwrap(),
        op_signature: hex::decode(v["signature_hex"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap(),
    }
    .encode();
    let red_at = ts + 100;
    let red = match fresh().open_frame(red_frame.clone(), red_at).unwrap() {
        Opened::RedNotice { view } => view,
        other => panic!("the Red op does not open as a Red notice: {other:?}"),
    };

    // kind 9
    let request = CertRequestWire {
        device_id: [0xcc; 8],
        scope: CertScope::Hosts(vec![[0x0a; 16]]),
        expiry: now + 7 * DAY,
        max_new_hosts_per_day: 2,
        epoch_lo: 0,
        epoch_hi: 1,
    }
    .encode();
    let core = fresh();
    let Opened::CertRequest { view: req } = core.open_frame(request.clone(), now).unwrap() else {
        panic!("the trust request does not open")
    };
    assert!(req.problems.is_empty(), "{:?}", req.problems);
    assert_eq!(req.type_to_approve.as_deref(), Some(DELEGATE));
    core.decide(
        req.token,
        Decision::Approve {
            typed_id: Some(DELEGATE.into()),
        },
        now,
    )
    .unwrap();
    let cert_id = core.issued_certs(now).unwrap()[0].cert_id.clone();

    // kind 10, on the phone that issued the certificate and on a fresh one
    let open_after = 60;
    let extend = CertExtendRequestWire {
        cert_id: hex::decode(&cert_id).unwrap().try_into().unwrap(),
        extra_hosts: 5,
    }
    .encode();
    let Opened::CertExtend { view: ext } =
        core.open_frame(extend.clone(), now + open_after).unwrap()
    else {
        panic!("the trust extension does not open")
    };
    assert!(ext.problems.is_empty(), "{:?}", ext.problems);
    let Opened::CertExtend { view: ext_fresh } = fresh().open_frame(extend.clone(), now).unwrap()
    else {
        panic!("the trust extension does not open on a fresh phone")
    };

    json!({
        "meta": {
            "note": "synthetic keys, not real; frames are what the terminal's encoders write",
            "regenerate": "SSHUB_EMIT_PHONE_FIXTURES=1 cargo test -p sshub-phone-core --test notices_fixture",
            "read_with": "the phone of challenges.json: secret 42 x 32 bytes, unlocked on a fresh sealed state",
        },
        "now": now,
        "phone": { "secret_hex": "42".repeat(32), "device_id": ch["phone"]["pub"]["device_id"] },
        "cases": [
            {
                "name": "red-notice",
                "kind": 3,
                "frame_hex": hex::encode(&red_frame),
                "open_at": red_at,
                "signer_ed25519_pub_hex": vectors["meta"]["signer_ed25519_pub_hex"],
                "expect": {
                    "opens_as": "red_notice",
                    "headline": red.op.headline,
                    "op_hash": red.op_hash,
                    "proposer": red.proposer,
                    "proposed_at": red.proposed_at,
                    "lapses_at": red.lapses_at,
                    "lamport": red.lamport,
                    "seq": red.seq,
                    "must_type": red.type_to_approve,
                    "danger_count": red.op.danger_count,
                },
            },
            {
                "name": "trust-request",
                "kind": 9,
                "frame_hex": hex::encode(&request),
                "expect": {
                    "opens_as": "cert_request",
                    "headline": req.headline,
                    "rows": rows(&req.rows),
                    "problems": req.problems,
                    "must_type": req.type_to_approve,
                },
                "approve": { "at": now, "typed_id": DELEGATE },
            },
            {
                "name": "trust-extension",
                "kind": 10,
                "frame_hex": hex::encode(&extend),
                "needs": "trust-request approved first",
                "cert_id_hex": cert_id,
                "open_after_secs": open_after,
                "expect": {
                    "opens_as": "cert_extend",
                    "headline": ext.headline,
                    "rows": rows(&ext.rows),
                    "problems": ext.problems,
                },
                "fresh_phone_problems": ext_fresh.problems,
            },
        ],
    })
}

#[test]
fn the_committed_notices_fixture_is_what_the_core_produces() {
    let mut text = serde_json::to_string_pretty(&build()).unwrap();
    text.push('\n');
    let path = format!("{FIX}notices.json");
    if std::env::var_os("SSHUB_EMIT_PHONE_FIXTURES").is_some() {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{path}: {e} (regenerate with SSHUB_EMIT_PHONE_FIXTURES=1)"));
    assert_eq!(committed, text, "notices.json is stale: regenerate it");
}
