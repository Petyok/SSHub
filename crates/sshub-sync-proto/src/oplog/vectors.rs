//! Committed canonical-op vectors (`tests/vectors/op/ops.json`).
//!
//! Every `OpBody` variant plus the encoding edge cases (u64::MAX and i64::MIN
//! literals, escapes, control characters, patch null versus absent). The file
//! carries the inputs in a logical form (byte strings as lowercase hex, keys in
//! alphabetical order), the canonical bytes, their BLAKE3 hash and an Ed25519
//! signature. `tests/oracles/op_canonical.py` rebuilds the bytes from the
//! inputs following only the contract text, so Rust and the oracle must agree
//! byte for byte. Regenerate: `SSHUB_EMIT_VECTORS=1 cargo test sync::oplog::vectors`.

use super::op::*;
use crate::crypto::{
    bip39_encode, paper_derive, DeviceIdentity, DevicePublic, PaperDomain, ALG_CURRENT,
};
use crate::uid::{uid_hex, Uid};
use serde_json::{json, Map, Value};

/// `Vec<u8>` fields: base64 in the canonical form, hex in the logical input.
const VEC_KEYS: [&str; 4] = ["mlkem_pub", "ciphertext", "sealed", "sealed_share"];

fn signer() -> DeviceIdentity {
    paper_derive(&bip39_encode(&[0x11; 32]), PaperDomain::Unwrap).unwrap()
}

fn peer(n: u8) -> DevicePublic {
    paper_derive(&bip39_encode(&[n; 32]), PaperDomain::Recover)
        .unwrap()
        .public()
}

fn u(n: u8) -> Uid {
    [n; 16]
}

fn hf() -> HostFields {
    HostFields::default()
}

fn cases() -> Vec<(&'static str, Op)> {
    let dev = signer().public().device_id();
    let other = "0011223344556677".to_string();
    let mk = |seq: u64, lamport: u64, timestamp: i64, epoch: u64, body: OpBody| Op {
        device_id: dev.clone(),
        seq,
        lamport,
        prev_hash: [seq as u8; 32],
        timestamp,
        epoch,
        alg: ALG_CURRENT,
        body,
        signature: [0; 64],
    };
    let share = |r: &str, sealed: Vec<u8>| WrappedShare {
        recipient: r.into(),
        alg: ALG_CURRENT,
        sealed,
    };
    let full_host = HostFields {
        name: Some("web-1".into()),
        label: Some(Some("Web \"one\"".into())),
        address: Some("web1.example".into()),
        port: Some(Some(2222)),
        username: Some(Some("deploy".into())),
        proxy_jump: Some(Some(format!("#{},jump.example:2200", uid_hex(&u(9))))),
        forward_agent: Some(Some(true)),
        remote_command: Some(Some("tmux attach".into())),
        transport: Some(Some("mosh".into())),
        environment: Some(Some("prod".into())),
        os_icon: Some(Some("linux".into())),
        tags: Some(vec!["a".into(), "b c".into()]),
        notes: Some(Some("line1\nline2\ttab".into())),
        favorite: Some(true),
        secret_lookup: Some(Some("service=ssh".into())),
        has_password: Some(true),
        group_uids: Some(vec![u(1), u(2)]),
        identity_uid: Some(Some(u(3))),
    };
    let cleared_host = HostFields {
        label: Some(None),
        port: Some(None),
        username: Some(None),
        proxy_jump: Some(None),
        forward_agent: Some(None),
        remote_command: Some(None),
        transport: Some(None),
        environment: Some(None),
        os_icon: Some(None),
        notes: Some(None),
        secret_lookup: Some(None),
        identity_uid: Some(None),
        tags: Some(vec![]),
        group_uids: Some(vec![]),
        ..hf()
    };
    vec![
        (
            "host-upsert-empty-patch",
            mk(
                1,
                1,
                1_893_452_400,
                0,
                OpBody::HostUpsert {
                    host_uid: u(1),
                    fields: hf(),
                },
            ),
        ),
        (
            "host-upsert-full",
            mk(
                2,
                2,
                1_893_452_401,
                0,
                OpBody::HostUpsert {
                    host_uid: u(2),
                    fields: full_host,
                },
            ),
        ),
        (
            "host-upsert-every-field-cleared",
            mk(
                3,
                3,
                0,
                0,
                OpBody::HostUpsert {
                    host_uid: u(2),
                    fields: cleared_host,
                },
            ),
        ),
        (
            "host-upsert-escapes-cyrillic-emoji",
            mk(
                4,
                4,
                -1,
                7,
                OpBody::HostUpsert {
                    host_uid: u(4),
                    fields: HostFields {
                        name: Some("Хост ⚡ 🚀 \"q\" \\ / \u{1} \u{1f} \u{7f} \u{2028} é".into()),
                        notes: Some(Some("\u{8}\u{c}\n\r\t".into())),
                        ..hf()
                    },
                },
            ),
        ),
        (
            "host-delete",
            mk(5, 5, 1, 0, OpBody::HostDelete { host_uid: u(5) }),
        ),
        (
            "group-upsert-full",
            mk(
                6,
                6,
                1,
                0,
                OpBody::GroupUpsert {
                    group_uid: u(6),
                    fields: GroupFields {
                        name: Some("prod".into()),
                        default_identity_uid: Some(Some(u(3))),
                        default_username: Some(Some("ops".into())),
                        default_port: Some(Some(22)),
                        default_proxy_jump: Some(Some("none".into())),
                        default_transport: Some(Some("ssh".into())),
                        default_forward_agent: Some(Some(false)),
                        parent_uid: Some(Some(u(7))),
                    },
                },
            ),
        ),
        (
            "group-upsert-cleared",
            mk(
                7,
                7,
                1,
                0,
                OpBody::GroupUpsert {
                    group_uid: u(6),
                    fields: GroupFields {
                        default_identity_uid: Some(None),
                        default_username: Some(None),
                        default_port: Some(None),
                        default_proxy_jump: Some(None),
                        default_transport: Some(None),
                        default_forward_agent: Some(None),
                        parent_uid: Some(None),
                        ..Default::default()
                    },
                },
            ),
        ),
        (
            "group-delete",
            mk(8, 8, 1, 0, OpBody::GroupDelete { group_uid: u(6) }),
        ),
        (
            "identity-upsert",
            mk(
                9,
                9,
                1,
                0,
                OpBody::IdentityUpsert {
                    identity_uid: u(3),
                    fields: IdentityFields {
                        name: Some("work".into()),
                        username: Some(None),
                        has_password: Some(true),
                    },
                },
            ),
        ),
        (
            "identity-delete",
            mk(10, 10, 1, 0, OpBody::IdentityDelete { identity_uid: u(3) }),
        ),
        (
            "cred-update",
            mk(
                11,
                11,
                1,
                1,
                OpBody::CredUpdate {
                    host_uid: u(2),
                    ciphertext: (0..=255).collect(),
                    wrapped_shares: vec![share(&dev, vec![1, 2, 3]), share(&other, vec![])],
                },
            ),
        ),
        (
            "cred-update-empty",
            mk(
                12,
                12,
                1,
                1,
                OpBody::CredUpdate {
                    host_uid: u(2),
                    ciphertext: vec![],
                    wrapped_shares: vec![],
                },
            ),
        ),
        (
            "device-add-mirror",
            mk(
                13,
                13,
                1,
                0,
                OpBody::DeviceAdd {
                    device: peer(0x21),
                    name: "NAS \u{1F5C4}".into(),
                    role: DeviceRole::Mirror,
                    attendance: Attendance::Unattended,
                    kind: DeviceKind::Terminal,
                    verified_by: VerifiedBy::SshHostKey,
                },
            ),
        ),
        (
            "device-add-holder",
            mk(
                14,
                14,
                1,
                0,
                OpBody::DeviceAdd {
                    device: peer(0x22),
                    name: "laptop".into(),
                    role: DeviceRole::Holder,
                    attendance: Attendance::Attended,
                    kind: DeviceKind::Phone,
                    verified_by: VerifiedBy::Sas,
                },
            ),
        ),
        (
            "device-revoke",
            mk(
                15,
                15,
                1,
                0,
                OpBody::DeviceRevoke {
                    device_id: other.clone(),
                },
            ),
        ),
        (
            "share-release-u64-max",
            mk(
                16,
                16,
                1,
                0,
                OpBody::ShareRelease {
                    host_uid: u(2),
                    request_id: u64::MAX,
                    requester: dev.clone(),
                    eph_x25519_pub: [0x9e; 32],
                },
            ),
        ),
        (
            "share-grant",
            mk(
                39,
                39,
                1,
                2,
                OpBody::ShareGrant {
                    host_uid: u(2),
                    request_id: u64::MAX,
                    requester: other.clone(),
                    epoch: 3,
                    sealed: (0..128).collect(),
                },
            ),
        ),
        (
            "share-grant-empty",
            mk(
                40,
                40,
                1,
                2,
                OpBody::ShareGrant {
                    host_uid: u(2),
                    request_id: 0,
                    requester: dev.clone(),
                    epoch: 0,
                    sealed: vec![],
                },
            ),
        ),
        (
            "share-reshare-session",
            mk(
                17,
                17,
                1,
                2,
                OpBody::ShareReshareSession {
                    epoch: 2,
                    to: other.clone(),
                    sealed: (0..=255).rev().collect(),
                },
            ),
        ),
        (
            "share-storage-key-empty",
            mk(
                18,
                18,
                1,
                2,
                OpBody::ShareStorageKey {
                    epoch: u64::MAX,
                    sealed: vec![],
                },
            ),
        ),
        (
            "share-reshare-batch",
            mk(
                31,
                31,
                1,
                2,
                OpBody::ShareReshareBatch {
                    epoch: 2,
                    new_holders: vec![dev.clone(), other.clone()],
                    items: vec![
                        ReshareItem {
                            host_uid: u(1),
                            sub_shares: vec![
                                (dev.clone(), vec![1, 2, 3, 4]),
                                (other.clone(), vec![0xfb, 0xff]),
                            ],
                            commitments: vec![vec![1, 2, 3], vec![], vec![255; 33]],
                        },
                        ReshareItem {
                            host_uid: u(2),
                            sub_shares: vec![],
                            commitments: vec![],
                        },
                    ],
                },
            ),
        ),
        (
            "share-reshare-batch-empty",
            mk(
                32,
                32,
                1,
                2,
                OpBody::ShareReshareBatch {
                    epoch: 0,
                    new_holders: vec![],
                    items: vec![],
                },
            ),
        ),
        (
            "share-reshare-confirm-batch",
            mk(
                33,
                33,
                1,
                2,
                OpBody::ShareReshareConfirmBatch {
                    epoch: 2,
                    items: vec![
                        ConfirmItem {
                            host_uid: u(1),
                            sealed_share: vec![7; 49],
                        },
                        ConfirmItem {
                            host_uid: u(2),
                            sealed_share: vec![],
                        },
                    ],
                },
            ),
        ),
        (
            "wrap-key-rotate",
            mk(
                34,
                34,
                1,
                2,
                OpBody::WrapKeyRotate {
                    device_id: dev.clone(),
                    wrap_epoch: 1,
                    x25519_pub: peer(0x23).x25519_pub,
                    mlkem_pub: peer(0x23).mlkem_pub,
                },
            ),
        ),
        (
            "cert-issue-hosts",
            mk(
                19,
                19,
                1,
                2,
                OpBody::CertIssue {
                    device_id: other.clone(),
                    scope: CertScope::Hosts(vec![u(1), u(2)]),
                    expiry: 1_900_000_000,
                    max_new_hosts_per_day: 50,
                    epoch_lo: 0,
                    epoch_hi: 5,
                },
            ),
        ),
        (
            "cert-issue-group-extremes",
            mk(
                20,
                20,
                1,
                2,
                OpBody::CertIssue {
                    device_id: other.clone(),
                    scope: CertScope::Group(u(6)),
                    expiry: i64::MIN,
                    max_new_hosts_per_day: u32::MAX,
                    epoch_lo: u64::MAX,
                    epoch_hi: u64::MAX,
                },
            ),
        ),
        (
            "cert-revoke",
            mk(
                21,
                21,
                1,
                2,
                OpBody::CertRevoke {
                    cert_id: [0x0c; 32],
                },
            ),
        ),
        (
            "cert-host-opened",
            mk(
                22,
                22,
                1,
                2,
                OpBody::CertHostOpened {
                    cert_id: [0x0c; 32],
                    host_uid: u(2),
                },
            ),
        ),
        (
            "phone-reenroll-hardware-key",
            mk(
                23,
                23,
                1,
                2,
                OpBody::PhoneReenroll {
                    initiated_at: 1_893_452_400,
                    signers: vec![dev.clone(), other.clone()],
                    via_paper: false,
                    third_factor: ThirdFactor::Paper,
                    liveness: vec![
                        HeadAttest {
                            device_id: dev.clone(),
                            seq: 3,
                            chain_hash: [1; 32],
                        },
                        HeadAttest {
                            device_id: other.clone(),
                            seq: u64::MAX,
                            chain_hash: [2; 32],
                        },
                    ],
                    paper_sig: Some([0xa5; 64]),
                },
            ),
        ),
        (
            "phone-reenroll-paper-empty",
            mk(
                24,
                24,
                1,
                2,
                OpBody::PhoneReenroll {
                    initiated_at: -1,
                    signers: vec![],
                    via_paper: true,
                    third_factor: ThirdFactor::Paper,
                    liveness: vec![],
                    paper_sig: None,
                },
            ),
        ),
        (
            "phone-reenroll-email-code",
            mk(
                25,
                25,
                1,
                2,
                OpBody::PhoneReenroll {
                    initiated_at: 0,
                    signers: vec![dev.clone()],
                    via_paper: false,
                    third_factor: ThirdFactor::EmailCode,
                    liveness: vec![],
                    paper_sig: None,
                },
            ),
        ),
        (
            "phone-reenroll-cancel-some",
            mk(
                26,
                26,
                1,
                2,
                OpBody::PhoneReenrollCancel {
                    reenroll_hash: [3; 32],
                    by_phone: false,
                    third_factor: Some(ThirdFactor::Paper),
                },
            ),
        ),
        (
            "phone-reenroll-cancel-none",
            mk(
                27,
                27,
                1,
                2,
                OpBody::PhoneReenrollCancel {
                    reenroll_hash: [3; 32],
                    by_phone: true,
                    third_factor: None,
                },
            ),
        ),
        (
            "recovery-ack",
            mk(
                28,
                28,
                1,
                2,
                OpBody::RecoveryAck {
                    reenroll_hash: [3; 32],
                    seen_at: 1_893_452_999,
                },
            ),
        ),
        (
            "mode-change-some",
            mk(
                29,
                29,
                1,
                2,
                OpBody::ModeChange {
                    to: CredMode::Quorum,
                    migrated_on: dev.clone(),
                    cosigner: Some(other.clone()),
                },
            ),
        ),
        (
            "mode-change-none",
            mk(
                30,
                30,
                1,
                2,
                OpBody::ModeChange {
                    to: CredMode::Vault,
                    migrated_on: dev.clone(),
                    cosigner: None,
                },
            ),
        ),
        (
            "recovery-approve",
            mk(
                35,
                35,
                1,
                2,
                OpBody::RecoveryApprove {
                    reenroll_hash: [0x77; 32],
                },
            ),
        ),
        (
            "red-approve",
            mk(
                41,
                41,
                1,
                2,
                OpBody::RedApprove {
                    op_hash: [0x61; 32],
                },
            ),
        ),
        (
            "red-phone-sign",
            mk(
                42,
                42,
                1,
                2,
                OpBody::RedPhoneSign {
                    op_hash: [0x62; 32],
                },
            ),
        ),
        (
            "red-factor",
            mk(
                43,
                43,
                1,
                2,
                OpBody::RedFactor {
                    op_hash: [0x63; 32],
                    factor: ThirdFactor::EmailCode,
                    sig: [0xc3; 64],
                },
            ),
        ),
        (
            "factor-register-paper",
            mk(
                44,
                44,
                1,
                2,
                OpBody::FactorRegister {
                    factor: ThirdFactor::Paper,
                    public: [0x64; 32],
                    sealed: vec![],
                    salt: [0x0f; 16],
                    sender: None,
                },
            ),
        ),
        (
            "factor-register-email",
            mk(
                45,
                45,
                1,
                2,
                OpBody::FactorRegister {
                    factor: ThirdFactor::EmailCode,
                    public: [0x65; 32],
                    sealed: (0..128).collect(),
                    salt: [0xf0; 16],
                    sender: Some(other.clone()),
                },
            ),
        ),
        (
            "device-role-change",
            mk(
                46,
                46,
                1,
                2,
                OpBody::DeviceRoleChange {
                    device_id: other.clone(),
                    to: DeviceRole::Holder,
                },
            ),
        ),
        (
            "exemption-set-on",
            mk(
                48,
                48,
                1,
                2,
                OpBody::ExemptionSet {
                    device_id: other.clone(),
                    slot: Some(peer(0x24)),
                },
            ),
        ),
        (
            "exemption-set-off",
            mk(
                49,
                49,
                1,
                2,
                OpBody::ExemptionSet {
                    device_id: other.clone(),
                    slot: None,
                },
            ),
        ),
        (
            "cert-extend",
            mk(
                50,
                50,
                1,
                2,
                OpBody::CertExtend {
                    cert_id: [0x66; 32],
                    extra_hosts: 50,
                },
            ),
        ),
        (
            "bulk-export",
            mk(
                47,
                47,
                1,
                2,
                OpBody::BulkExport {
                    requester: dev.clone(),
                    window_secs: 3600,
                },
            ),
        ),
        (
            "checkpoint-propose",
            mk(
                36,
                36,
                1,
                2,
                OpBody::CheckpointPropose {
                    cut: vec![
                        CutEntry {
                            device_id: dev.clone(),
                            seq: 35,
                            op_hash: [0x11; 32],
                            lamport: 35,
                        },
                        CutEntry {
                            device_id: other.clone(),
                            seq: u64::MAX,
                            op_hash: [0x22; 32],
                            lamport: u64::MAX,
                        },
                    ],
                    state_hash: [0x33; 32],
                    state_len: 612_345,
                    prev_checkpoint: [0; 32],
                    required: vec![dev.clone(), other.clone()],
                },
            ),
        ),
        (
            "checkpoint-propose-empty",
            mk(
                37,
                37,
                1,
                2,
                OpBody::CheckpointPropose {
                    cut: vec![],
                    state_hash: [0; 32],
                    state_len: 0,
                    prev_checkpoint: [0x44; 32],
                    required: vec![],
                },
            ),
        ),
        (
            "checkpoint-sign",
            mk(
                38,
                38,
                1,
                2,
                OpBody::CheckpointSign {
                    proposal_hash: [0x55; 32],
                },
            ),
        ),
        (
            "head-attest-u64-max",
            mk(
                u64::MAX,
                18446744073709551615,
                i64::MIN,
                u64::MAX,
                OpBody::HeadAttest {
                    device_id: other,
                    seq: u64::MAX,
                    chain_hash: [4; 32],
                },
            ),
        ),
    ]
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Canonical JSON with `Vec<u8>` fields turned into hex, so the oracle input
/// does not repeat the encoding it has to rebuild (fixed 32-byte values are hex
/// in the canonical form already).
fn logical(v: Value, key: &str) -> Value {
    let unb64 = |x: &Value| {
        Value::String(hex(&crate::oplog::canonical::b64_decode(
            x.as_str().unwrap(),
        )
        .unwrap()))
    };
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let v = logical(v, &k);
                    (k, v)
                })
                .collect::<Map<_, _>>(),
        ),
        Value::String(_) if VEC_KEYS.contains(&key) => unb64(&v),
        Value::Array(a) if key == "commitments" => Value::Array(a.iter().map(unb64).collect()),
        Value::Array(a) if key == "sub_shares" => Value::Array(
            a.into_iter()
                .map(|p| {
                    let p = p.as_array().unwrap();
                    Value::Array(vec![p[0].clone(), unb64(&p[1])])
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(|x| logical(x, key)).collect()),
        other => other,
    }
}

fn build() -> Value {
    let id = signer();
    let vectors: Vec<Value> = cases()
        .into_iter()
        .map(|(name, mut op)| {
            let canonical = op.canonical_bytes();
            op.signature = id.sign(&canonical);
            let input: Value = serde_json::from_slice(&canonical).unwrap();
            json!({
                "id": name,
                "op": logical(input, ""),
                "canonical_hex": hex(&canonical),
                "hash_hex": hex(&op.hash()),
                "signature_hex": hex(&op.signature),
            })
        })
        .collect();
    json!({
        "meta": {
            "contract": "docs/host-sync-contract.md section 0 (C2) and section 2",
            "note": "Inputs use alphabetical keys and hex for byte strings; the canonical form is rebuilt by tests/oracles/op_canonical.py from the contract text alone. Keys are synthetic (paper phrases of byte patterns).",
            "regenerate": "SSHUB_EMIT_VECTORS=1 cargo test sync::oplog::vectors",
            "signer_ed25519_pub_hex": hex(&id.public().ed25519_pub),
        },
        "vectors": vectors,
    })
}

fn path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/vectors/op/ops.json")
}

#[test]
fn committed_op_vectors_match_the_encoder_and_verify() {
    let fresh = build();
    if std::env::var("SSHUB_EMIT_VECTORS").is_ok() {
        std::fs::create_dir_all(path().parent().unwrap()).unwrap();
        std::fs::write(path(), serde_json::to_string_pretty(&fresh).unwrap() + "\n").unwrap();
    }
    let committed: Value = serde_json::from_str(&std::fs::read_to_string(path()).unwrap()).unwrap();
    assert_eq!(
        committed, fresh,
        "vectors are stale: regenerate and review the diff"
    );
    let signer_pub: [u8; 32] = {
        let h = committed["meta"]["signer_ed25519_pub_hex"]
            .as_str()
            .unwrap();
        (0..32)
            .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    };
    let unhex = |s: &str| -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    };
    for v in committed["vectors"].as_array().unwrap() {
        let canonical = unhex(v["canonical_hex"].as_str().unwrap());
        let sig: [u8; 64] = unhex(v["signature_hex"].as_str().unwrap())
            .try_into()
            .unwrap();
        let op = Op::from_canonical(&canonical, sig).unwrap();
        assert_eq!(op.canonical_bytes(), canonical, "{}", v["id"]);
        assert_eq!(
            hex(&op.hash()),
            v["hash_hex"].as_str().unwrap(),
            "{}",
            v["id"]
        );
        assert!(op.verify(&signer_pub), "{}", v["id"]);
    }
    assert!(committed["vectors"].as_array().unwrap().len() >= 30);
}

#[test]
fn every_body_variant_has_a_vector() {
    let names: std::collections::BTreeSet<&str> = cases()
        .iter()
        .map(|(_, op)| super::canonical::body_name(&op.body))
        .collect();
    assert_eq!(names.len(), 35, "{names:?}");
}
