//! Canonical op bytes (contract C2): deterministic compact JSON written by an
//! explicit, hand-ordered encoder (never by `#[derive(Serialize)]` field order).
//!
//! Header order: `device_id, seq, lamport, prev_hash, timestamp, epoch, alg,
//! body`. `prev_hash` is lowercase hex; uids are 32 lowercase hex characters;
//! every other byte string (`[u8; 32]` hashes, ciphertexts, public keys) is a
//! JSON array of numbers; `alg` is the two-byte wire form `[sig, wrap]`;
//! integers are plain literals (`u64::MAX` is `18446744073709551615`); strings
//! are raw UTF-8 with only the mandatory JSON escapes. The body is externally
//! tagged: `{"Variant":{fields in contract order}}`. Patch fields (`HostFields`
//! and friends) omit absent keys and write `null` for "cleared".
//!
//! The decoder is strict: it accepts exactly the bytes the encoder writes.

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde_json::{Map, Value};
use std::io::Write;

use super::op::*;
use crate::store::{uid_from_hex, uid_hex, Uid};
use crate::sync::crypto::{AlgTag, DevicePublic};

// ---------------------------------------------------------------- encoding

struct Obj<'a> {
    b: &'a mut Vec<u8>,
    n: usize,
}

impl<'a> Obj<'a> {
    fn new(b: &'a mut Vec<u8>) -> Self {
        b.push(b'{');
        Self { b, n: 0 }
    }

    fn key(&mut self, k: &str) -> &mut Vec<u8> {
        if self.n > 0 {
            self.b.push(b',');
        }
        self.n += 1;
        string(self.b, k);
        self.b.push(b':');
        self.b
    }

    fn end(self) {
        self.b.push(b'}');
    }
}

fn string(b: &mut Vec<u8>, s: &str) {
    serde_json::to_writer(&mut *b, s).expect("string serialises");
}

fn value(b: &mut Vec<u8>, v: &Value) {
    serde_json::to_writer(&mut *b, v).expect("value serialises");
}

fn bytes_arr(b: &mut Vec<u8>, bytes: &[u8]) {
    b.push(b'[');
    for (i, x) in bytes.iter().enumerate() {
        if i > 0 {
            b.push(b',');
        }
        write!(b, "{x}").expect("vec write");
    }
    b.push(b']');
}

fn uid_str(b: &mut Vec<u8>, u: &Uid) {
    string(b, &uid_hex(u));
}

fn opt_str(b: &mut Vec<u8>, s: &Option<String>) {
    match s {
        Some(s) => string(b, s),
        None => b.extend_from_slice(b"null"),
    }
}

fn strings(b: &mut Vec<u8>, items: &[String]) {
    b.push(b'[');
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            b.push(b',');
        }
        string(b, s);
    }
    b.push(b']');
}

fn alg(b: &mut Vec<u8>, a: AlgTag) {
    bytes_arr(b, &a.to_bytes());
}

fn third_factor(b: &mut Vec<u8>, t: &ThirdFactor) {
    match t {
        ThirdFactor::Paper => string(b, "Paper"),
        ThirdFactor::EmailCode => string(b, "EmailCode"),
        ThirdFactor::HardwareKey(id) => {
            let mut o = Obj::new(b);
            string(o.key("HardwareKey"), id);
            o.end();
        }
    }
}

fn head_attest(b: &mut Vec<u8>, h: &HeadAttest) {
    let mut o = Obj::new(b);
    string(o.key("device_id"), &h.device_id);
    write!(o.key("seq"), "{}", h.seq).expect("vec write");
    bytes_arr(o.key("chain_hash"), &h.chain_hash);
    o.end();
}

fn patch_fields(b: &mut Vec<u8>, fields: &[(&'static str, Value)]) {
    let mut o = Obj::new(b);
    for (k, v) in fields {
        value(o.key(k), v);
    }
    o.end();
}

/// Variant name of a body, as it appears in the canonical form and in
/// `sync_ops.kind`.
pub fn body_name(body: &OpBody) -> &'static str {
    match body {
        OpBody::HostUpsert { .. } => "HostUpsert",
        OpBody::HostDelete { .. } => "HostDelete",
        OpBody::GroupUpsert { .. } => "GroupUpsert",
        OpBody::GroupDelete { .. } => "GroupDelete",
        OpBody::IdentityUpsert { .. } => "IdentityUpsert",
        OpBody::IdentityDelete { .. } => "IdentityDelete",
        OpBody::CredUpdate { .. } => "CredUpdate",
        OpBody::DeviceAdd { .. } => "DeviceAdd",
        OpBody::DeviceRevoke { .. } => "DeviceRevoke",
        OpBody::ShareRelease { .. } => "ShareRelease",
        OpBody::ShareReshare { .. } => "ShareReshare",
        OpBody::ShareReshareConfirm { .. } => "ShareReshareConfirm",
        OpBody::CertIssue { .. } => "CertIssue",
        OpBody::CertRevoke { .. } => "CertRevoke",
        OpBody::CertHostOpened { .. } => "CertHostOpened",
        OpBody::PhoneReenroll { .. } => "PhoneReenroll",
        OpBody::PhoneReenrollCancel { .. } => "PhoneReenrollCancel",
        OpBody::RecoveryAck { .. } => "RecoveryAck",
        OpBody::ModeChange { .. } => "ModeChange",
        OpBody::HeadAttest { .. } => "HeadAttest",
    }
}

fn body(b: &mut Vec<u8>, body: &OpBody) {
    let mut outer = Obj::new(b);
    let b = outer.key(body_name(body));
    let mut o = Obj::new(b);
    match body {
        OpBody::HostUpsert { host_uid, fields } => {
            uid_str(o.key("host_uid"), host_uid);
            patch_fields(o.key("fields"), &host_field_values(fields));
        }
        OpBody::GroupUpsert { group_uid, fields } => {
            uid_str(o.key("group_uid"), group_uid);
            patch_fields(o.key("fields"), &group_field_values(fields));
        }
        OpBody::IdentityUpsert {
            identity_uid,
            fields,
        } => {
            uid_str(o.key("identity_uid"), identity_uid);
            patch_fields(o.key("fields"), &identity_field_values(fields));
        }
        OpBody::HostDelete { host_uid } => uid_str(o.key("host_uid"), host_uid),
        OpBody::GroupDelete { group_uid } => uid_str(o.key("group_uid"), group_uid),
        OpBody::IdentityDelete { identity_uid } => uid_str(o.key("identity_uid"), identity_uid),
        OpBody::CredUpdate {
            host_uid,
            ciphertext,
            wrapped_shares,
        } => {
            uid_str(o.key("host_uid"), host_uid);
            bytes_arr(o.key("ciphertext"), ciphertext);
            let b = o.key("wrapped_shares");
            b.push(b'[');
            for (i, s) in wrapped_shares.iter().enumerate() {
                if i > 0 {
                    b.push(b',');
                }
                let mut so = Obj::new(b);
                string(so.key("recipient"), &s.recipient);
                alg(so.key("alg"), s.alg);
                bytes_arr(so.key("sealed"), &s.sealed);
                so.end();
            }
            b.push(b']');
        }
        OpBody::DeviceAdd {
            device,
            name,
            role,
            attendance,
        } => {
            let mut d = Obj::new(o.key("device"));
            bytes_arr(d.key("ed25519_pub"), &device.ed25519_pub);
            bytes_arr(d.key("x25519_pub"), &device.x25519_pub);
            bytes_arr(d.key("mlkem_pub"), &device.mlkem_pub);
            d.end();
            string(o.key("name"), name);
            string(o.key("role"), role_str(*role));
            string(o.key("attendance"), attendance_str(*attendance));
        }
        OpBody::DeviceRevoke { device_id } => string(o.key("device_id"), device_id),
        OpBody::ShareRelease {
            host_uid,
            request_id,
            requester,
        } => {
            uid_str(o.key("host_uid"), host_uid);
            write!(o.key("request_id"), "{request_id}").expect("vec write");
            string(o.key("requester"), requester);
        }
        OpBody::ShareReshare {
            epoch,
            new_holders,
            commitments,
        } => {
            write!(o.key("epoch"), "{epoch}").expect("vec write");
            strings(o.key("new_holders"), new_holders);
            let b = o.key("commitments");
            b.push(b'[');
            for (i, c) in commitments.iter().enumerate() {
                if i > 0 {
                    b.push(b',');
                }
                bytes_arr(b, c);
            }
            b.push(b']');
        }
        OpBody::ShareReshareConfirm { reshare_hash } => {
            bytes_arr(o.key("reshare_hash"), reshare_hash)
        }
        OpBody::CertIssue {
            device_id,
            scope,
            expiry,
            max_new_hosts_per_day,
            epoch_lo,
            epoch_hi,
        } => {
            string(o.key("device_id"), device_id);
            let mut s = Obj::new(o.key("scope"));
            match scope {
                CertScope::Group(g) => uid_str(s.key("Group"), g),
                CertScope::Hosts(hosts) => {
                    let b = s.key("Hosts");
                    b.push(b'[');
                    for (i, h) in hosts.iter().enumerate() {
                        if i > 0 {
                            b.push(b',');
                        }
                        uid_str(b, h);
                    }
                    b.push(b']');
                }
            }
            s.end();
            write!(o.key("expiry"), "{expiry}").expect("vec write");
            write!(o.key("max_new_hosts_per_day"), "{max_new_hosts_per_day}").expect("vec write");
            write!(o.key("epoch_lo"), "{epoch_lo}").expect("vec write");
            write!(o.key("epoch_hi"), "{epoch_hi}").expect("vec write");
        }
        OpBody::CertRevoke { cert_id } => bytes_arr(o.key("cert_id"), cert_id),
        OpBody::CertHostOpened { cert_id, host_uid } => {
            bytes_arr(o.key("cert_id"), cert_id);
            uid_str(o.key("host_uid"), host_uid);
        }
        OpBody::PhoneReenroll {
            initiated_at,
            signers,
            via_paper,
            third_factor: tf,
            liveness,
        } => {
            write!(o.key("initiated_at"), "{initiated_at}").expect("vec write");
            strings(o.key("signers"), signers);
            write!(o.key("via_paper"), "{via_paper}").expect("vec write");
            third_factor(o.key("third_factor"), tf);
            let b = o.key("liveness");
            b.push(b'[');
            for (i, h) in liveness.iter().enumerate() {
                if i > 0 {
                    b.push(b',');
                }
                head_attest(b, h);
            }
            b.push(b']');
        }
        OpBody::PhoneReenrollCancel {
            reenroll_hash,
            by_phone,
            third_factor: tf,
        } => {
            bytes_arr(o.key("reenroll_hash"), reenroll_hash);
            write!(o.key("by_phone"), "{by_phone}").expect("vec write");
            let b = o.key("third_factor");
            match tf {
                Some(t) => third_factor(b, t),
                None => b.extend_from_slice(b"null"),
            }
        }
        OpBody::RecoveryAck {
            reenroll_hash,
            seen_at,
        } => {
            bytes_arr(o.key("reenroll_hash"), reenroll_hash);
            write!(o.key("seen_at"), "{seen_at}").expect("vec write");
        }
        OpBody::ModeChange {
            to,
            migrated_on,
            cosigner,
        } => {
            string(
                o.key("to"),
                match to {
                    CredMode::Vault => "Vault",
                    CredMode::Quorum => "Quorum",
                },
            );
            string(o.key("migrated_on"), migrated_on);
            opt_str(o.key("cosigner"), cosigner);
        }
        OpBody::HeadAttest {
            device_id,
            seq,
            chain_hash,
        } => {
            string(o.key("device_id"), device_id);
            write!(o.key("seq"), "{seq}").expect("vec write");
            bytes_arr(o.key("chain_hash"), chain_hash);
        }
    }
    o.end();
    outer.end();
}

fn role_str(r: DeviceRole) -> &'static str {
    match r {
        DeviceRole::Holder => "Holder",
        DeviceRole::Mirror => "Mirror",
    }
}

fn attendance_str(a: Attendance) -> &'static str {
    match a {
        Attendance::Attended => "Attended",
        Attendance::Unattended => "Unattended",
    }
}

/// The canonical bytes of `op`: everything except the signature.
pub fn encode_op(op: &Op) -> Vec<u8> {
    let mut b = Vec::with_capacity(256);
    let mut o = Obj::new(&mut b);
    string(o.key("device_id"), &op.device_id);
    write!(o.key("seq"), "{}", op.seq).expect("vec write");
    write!(o.key("lamport"), "{}", op.lamport).expect("vec write");
    string(o.key("prev_hash"), &hex_lower(&op.prev_hash));
    write!(o.key("timestamp"), "{}", op.timestamp).expect("vec write");
    write!(o.key("epoch"), "{}", op.epoch).expect("vec write");
    alg(o.key("alg"), op.alg);
    body(o.key("body"), &op.body);
    o.end();
    b
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ------------------------------------------------------- field value lists

fn opt_value<T>(
    out: &mut Vec<(&'static str, Value)>,
    k: &'static str,
    v: &Option<Option<T>>,
    f: impl Fn(&T) -> Value,
) {
    if let Some(inner) = v {
        out.push((k, inner.as_ref().map_or(Value::Null, f)));
    }
}

fn req_value<T>(
    out: &mut Vec<(&'static str, Value)>,
    k: &'static str,
    v: &Option<T>,
    f: impl Fn(&T) -> Value,
) {
    if let Some(v) = v {
        out.push((k, f(v)));
    }
}

// Matches the `Fn(&T)` shape of `req_value`/`opt_value` for `T = String`.
#[allow(clippy::ptr_arg)]
fn s(v: &String) -> Value {
    Value::String(v.clone())
}

fn u(v: &Uid) -> Value {
    Value::String(uid_hex(v))
}

/// Present fields of a host patch as `(name, JSON value)` in encoding order.
/// The same pairs are what the register table stores.
pub fn host_field_values(f: &HostFields) -> Vec<(&'static str, Value)> {
    let mut o = Vec::new();
    req_value(&mut o, "name", &f.name, s);
    opt_value(&mut o, "label", &f.label, s);
    req_value(&mut o, "address", &f.address, s);
    opt_value(&mut o, "port", &f.port, |p| Value::from(*p));
    opt_value(&mut o, "username", &f.username, s);
    opt_value(&mut o, "proxy_jump", &f.proxy_jump, s);
    opt_value(&mut o, "forward_agent", &f.forward_agent, |v| {
        Value::Bool(*v)
    });
    opt_value(&mut o, "remote_command", &f.remote_command, s);
    opt_value(&mut o, "transport", &f.transport, s);
    opt_value(&mut o, "environment", &f.environment, s);
    opt_value(&mut o, "os_icon", &f.os_icon, s);
    req_value(&mut o, "tags", &f.tags, |t| {
        Value::Array(t.iter().map(s).collect())
    });
    opt_value(&mut o, "notes", &f.notes, s);
    req_value(&mut o, "favorite", &f.favorite, |v| Value::Bool(*v));
    opt_value(&mut o, "secret_lookup", &f.secret_lookup, s);
    req_value(&mut o, "has_password", &f.has_password, |v| Value::Bool(*v));
    req_value(&mut o, "group_uids", &f.group_uids, |g| {
        Value::Array(g.iter().map(u).collect())
    });
    opt_value(&mut o, "identity_uid", &f.identity_uid, u);
    o
}

pub fn group_field_values(f: &GroupFields) -> Vec<(&'static str, Value)> {
    let mut o = Vec::new();
    req_value(&mut o, "name", &f.name, s);
    opt_value(&mut o, "default_identity_uid", &f.default_identity_uid, u);
    opt_value(&mut o, "default_username", &f.default_username, s);
    opt_value(&mut o, "default_port", &f.default_port, |p| Value::from(*p));
    opt_value(&mut o, "default_proxy_jump", &f.default_proxy_jump, s);
    opt_value(&mut o, "default_transport", &f.default_transport, s);
    opt_value(
        &mut o,
        "default_forward_agent",
        &f.default_forward_agent,
        |v| Value::Bool(*v),
    );
    opt_value(&mut o, "parent_uid", &f.parent_uid, u);
    o
}

pub fn identity_field_values(f: &IdentityFields) -> Vec<(&'static str, Value)> {
    let mut o = Vec::new();
    req_value(&mut o, "name", &f.name, s);
    opt_value(&mut o, "username", &f.username, s);
    req_value(&mut o, "has_password", &f.has_password, |v| Value::Bool(*v));
    o
}

// ---------------------------------------------------------------- decoding

pub fn decode_op(bytes: &[u8], signature: [u8; 64]) -> Result<Op> {
    let v: Value = serde_json::from_slice(bytes).context("op is not JSON")?;
    let m = as_obj(&v)?;
    let op = Op {
        device_id: get_string(m, "device_id")?,
        seq: get_u64(m, "seq")?,
        lamport: get_u64(m, "lamport")?,
        prev_hash: {
            let h = get_string(m, "prev_hash")?;
            ensure!(
                h.len() == 64
                    && h.bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
                "prev_hash must be 64 lowercase hex characters"
            );
            let mut out = [0u8; 32];
            for (i, byte) in out.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&h[2 * i..2 * i + 2], 16)?;
            }
            out
        },
        timestamp: get_i64(m, "timestamp")?,
        epoch: get_u64(m, "epoch")?,
        alg: {
            let a: [u8; 2] = get_bytes(m, "alg")?
                .try_into()
                .map_err(|_| anyhow!("alg must be two bytes"))?;
            AlgTag::from_bytes(a)?
        },
        body: decode_body(get(m, "body")?)?,
        signature,
    };
    ensure!(
        encode_op(&op) == bytes,
        "op bytes are not in canonical form"
    );
    Ok(op)
}

fn as_obj(v: &Value) -> Result<&Map<String, Value>> {
    v.as_object().ok_or_else(|| anyhow!("expected an object"))
}

fn get<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a Value> {
    m.get(k).ok_or_else(|| anyhow!("missing field {k}"))
}

fn get_string(m: &Map<String, Value>, k: &str) -> Result<String> {
    str_of(get(m, k)?, k)
}

fn str_of(v: &Value, k: &str) -> Result<String> {
    v.as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("{k} must be a string"))
}

fn get_u64(m: &Map<String, Value>, k: &str) -> Result<u64> {
    get(m, k)?
        .as_u64()
        .ok_or_else(|| anyhow!("{k} must be an unsigned integer"))
}

fn get_i64(m: &Map<String, Value>, k: &str) -> Result<i64> {
    get(m, k)?
        .as_i64()
        .ok_or_else(|| anyhow!("{k} must be an integer"))
}

fn get_bool(m: &Map<String, Value>, k: &str) -> Result<bool> {
    get(m, k)?
        .as_bool()
        .ok_or_else(|| anyhow!("{k} must be a boolean"))
}

fn bytes_of(v: &Value, k: &str) -> Result<Vec<u8>> {
    v.as_array()
        .ok_or_else(|| anyhow!("{k} must be an array of bytes"))?
        .iter()
        .map(|x| {
            x.as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| anyhow!("{k} must hold bytes"))
        })
        .collect()
}

fn get_bytes(m: &Map<String, Value>, k: &str) -> Result<Vec<u8>> {
    bytes_of(get(m, k)?, k)
}

fn get_32(m: &Map<String, Value>, k: &str) -> Result<[u8; 32]> {
    get_bytes(m, k)?
        .try_into()
        .map_err(|_| anyhow!("{k} must be 32 bytes"))
}

fn uid_of(v: &Value, k: &str) -> Result<Uid> {
    uid_from_hex(
        v.as_str()
            .ok_or_else(|| anyhow!("{k} must be a uid string"))?,
    )
    .ok_or_else(|| anyhow!("{k} must be 32 lowercase hex characters"))
}

fn get_uid(m: &Map<String, Value>, k: &str) -> Result<Uid> {
    uid_of(get(m, k)?, k)
}

fn get_strings(m: &Map<String, Value>, k: &str) -> Result<Vec<String>> {
    strings_of(get(m, k)?, k)
}

fn strings_of(v: &Value, k: &str) -> Result<Vec<String>> {
    v.as_array()
        .ok_or_else(|| anyhow!("{k} must be an array"))?
        .iter()
        .map(|x| str_of(x, k))
        .collect()
}

fn get_array<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a Vec<Value>> {
    get(m, k)?
        .as_array()
        .ok_or_else(|| anyhow!("{k} must be an array"))
}

fn opt_string(m: &Map<String, Value>, k: &str) -> Result<Option<String>> {
    match get(m, k)? {
        Value::Null => Ok(None),
        v => Ok(Some(str_of(v, k)?)),
    }
}

fn third_factor_of(v: &Value) -> Result<ThirdFactor> {
    match v {
        Value::String(s) if s == "Paper" => Ok(ThirdFactor::Paper),
        Value::String(s) if s == "EmailCode" => Ok(ThirdFactor::EmailCode),
        Value::Object(m) if m.len() == 1 => {
            Ok(ThirdFactor::HardwareKey(get_string(m, "HardwareKey")?))
        }
        _ => bail!("unknown third factor"),
    }
}

fn decode_body(v: &Value) -> Result<OpBody> {
    let outer = as_obj(v)?;
    ensure!(outer.len() == 1, "body must hold exactly one variant");
    let (name, inner) = outer.iter().next().expect("one entry");
    let m = as_obj(inner)?;
    Ok(match name.as_str() {
        "HostUpsert" => OpBody::HostUpsert {
            host_uid: get_uid(m, "host_uid")?,
            fields: host_fields_of(as_obj(get(m, "fields")?)?)?,
        },
        "HostDelete" => OpBody::HostDelete {
            host_uid: get_uid(m, "host_uid")?,
        },
        "GroupUpsert" => OpBody::GroupUpsert {
            group_uid: get_uid(m, "group_uid")?,
            fields: group_fields_of(as_obj(get(m, "fields")?)?)?,
        },
        "GroupDelete" => OpBody::GroupDelete {
            group_uid: get_uid(m, "group_uid")?,
        },
        "IdentityUpsert" => OpBody::IdentityUpsert {
            identity_uid: get_uid(m, "identity_uid")?,
            fields: identity_fields_of(as_obj(get(m, "fields")?)?)?,
        },
        "IdentityDelete" => OpBody::IdentityDelete {
            identity_uid: get_uid(m, "identity_uid")?,
        },
        "CredUpdate" => OpBody::CredUpdate {
            host_uid: get_uid(m, "host_uid")?,
            ciphertext: get_bytes(m, "ciphertext")?,
            wrapped_shares: get_array(m, "wrapped_shares")?
                .iter()
                .map(|s| {
                    let s = as_obj(s)?;
                    let a: [u8; 2] = get_bytes(s, "alg")?
                        .try_into()
                        .map_err(|_| anyhow!("alg must be two bytes"))?;
                    Ok(WrappedShare {
                        recipient: get_string(s, "recipient")?,
                        alg: AlgTag::from_bytes(a)?,
                        sealed: get_bytes(s, "sealed")?,
                    })
                })
                .collect::<Result<_>>()?,
        },
        "DeviceAdd" => {
            let d = as_obj(get(m, "device")?)?;
            OpBody::DeviceAdd {
                device: DevicePublic {
                    ed25519_pub: get_32(d, "ed25519_pub")?,
                    x25519_pub: get_32(d, "x25519_pub")?,
                    mlkem_pub: get_bytes(d, "mlkem_pub")?,
                },
                name: get_string(m, "name")?,
                role: match get_string(m, "role")?.as_str() {
                    "Holder" => DeviceRole::Holder,
                    "Mirror" => DeviceRole::Mirror,
                    r => bail!("unknown role {r}"),
                },
                attendance: match get_string(m, "attendance")?.as_str() {
                    "Attended" => Attendance::Attended,
                    "Unattended" => Attendance::Unattended,
                    a => bail!("unknown attendance {a}"),
                },
            }
        }
        "DeviceRevoke" => OpBody::DeviceRevoke {
            device_id: get_string(m, "device_id")?,
        },
        "ShareRelease" => OpBody::ShareRelease {
            host_uid: get_uid(m, "host_uid")?,
            request_id: get_u64(m, "request_id")?,
            requester: get_string(m, "requester")?,
        },
        "ShareReshare" => OpBody::ShareReshare {
            epoch: get_u64(m, "epoch")?,
            new_holders: get_strings(m, "new_holders")?,
            commitments: get_array(m, "commitments")?
                .iter()
                .map(|c| bytes_of(c, "commitments"))
                .collect::<Result<_>>()?,
        },
        "ShareReshareConfirm" => OpBody::ShareReshareConfirm {
            reshare_hash: get_32(m, "reshare_hash")?,
        },
        "CertIssue" => {
            let sc = as_obj(get(m, "scope")?)?;
            ensure!(sc.len() == 1, "scope must hold exactly one variant");
            let scope = match sc.iter().next().expect("one entry") {
                (k, v) if k == "Group" => CertScope::Group(uid_of(v, "scope")?),
                (k, v) if k == "Hosts" => CertScope::Hosts(
                    v.as_array()
                        .ok_or_else(|| anyhow!("scope hosts must be an array"))?
                        .iter()
                        .map(|h| uid_of(h, "scope"))
                        .collect::<Result<_>>()?,
                ),
                _ => bail!("unknown scope"),
            };
            OpBody::CertIssue {
                device_id: get_string(m, "device_id")?,
                scope,
                expiry: get_i64(m, "expiry")?,
                max_new_hosts_per_day: u32::try_from(get_u64(m, "max_new_hosts_per_day")?)?,
                epoch_lo: get_u64(m, "epoch_lo")?,
                epoch_hi: get_u64(m, "epoch_hi")?,
            }
        }
        "CertRevoke" => OpBody::CertRevoke {
            cert_id: get_32(m, "cert_id")?,
        },
        "CertHostOpened" => OpBody::CertHostOpened {
            cert_id: get_32(m, "cert_id")?,
            host_uid: get_uid(m, "host_uid")?,
        },
        "PhoneReenroll" => OpBody::PhoneReenroll {
            initiated_at: get_i64(m, "initiated_at")?,
            signers: get_strings(m, "signers")?,
            via_paper: get_bool(m, "via_paper")?,
            third_factor: third_factor_of(get(m, "third_factor")?)?,
            liveness: get_array(m, "liveness")?
                .iter()
                .map(|h| {
                    let h = as_obj(h)?;
                    Ok(HeadAttest {
                        device_id: get_string(h, "device_id")?,
                        seq: get_u64(h, "seq")?,
                        chain_hash: get_32(h, "chain_hash")?,
                    })
                })
                .collect::<Result<_>>()?,
        },
        "PhoneReenrollCancel" => OpBody::PhoneReenrollCancel {
            reenroll_hash: get_32(m, "reenroll_hash")?,
            by_phone: get_bool(m, "by_phone")?,
            third_factor: match get(m, "third_factor")? {
                Value::Null => None,
                v => Some(third_factor_of(v)?),
            },
        },
        "RecoveryAck" => OpBody::RecoveryAck {
            reenroll_hash: get_32(m, "reenroll_hash")?,
            seen_at: get_i64(m, "seen_at")?,
        },
        "ModeChange" => OpBody::ModeChange {
            to: match get_string(m, "to")?.as_str() {
                "Vault" => CredMode::Vault,
                "Quorum" => CredMode::Quorum,
                t => bail!("unknown mode {t}"),
            },
            migrated_on: get_string(m, "migrated_on")?,
            cosigner: opt_string(m, "cosigner")?,
        },
        "HeadAttest" => OpBody::HeadAttest {
            device_id: get_string(m, "device_id")?,
            seq: get_u64(m, "seq")?,
            chain_hash: get_32(m, "chain_hash")?,
        },
        other => bail!("unknown op body {other}"),
    })
}

/// Patch reader: absent key = `None`, `null` = `Some(None)`.
fn patch<T>(
    m: &Map<String, Value>,
    k: &str,
    f: impl Fn(&Value) -> Result<T>,
) -> Result<Option<Option<T>>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => Ok(Some(Some(f(v)?))),
    }
}

/// Non-nullable patch field: `null` is refused.
fn req<T>(m: &Map<String, Value>, k: &str, f: impl Fn(&Value) -> Result<T>) -> Result<Option<T>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::Null) => bail!("{k} cannot be null"),
        Some(v) => Ok(Some(f(v)?)),
    }
}

fn port_of(v: &Value) -> Result<u16> {
    v.as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .ok_or_else(|| anyhow!("port must fit in u16"))
}

fn bool_of(v: &Value) -> Result<bool> {
    v.as_bool().ok_or_else(|| anyhow!("expected a boolean"))
}

fn host_fields_of(m: &Map<String, Value>) -> Result<HostFields> {
    let st = |v: &Value| str_of(v, "field");
    Ok(HostFields {
        name: req(m, "name", st)?,
        label: patch(m, "label", st)?,
        address: req(m, "address", st)?,
        port: patch(m, "port", port_of)?,
        username: patch(m, "username", st)?,
        proxy_jump: patch(m, "proxy_jump", st)?,
        forward_agent: patch(m, "forward_agent", bool_of)?,
        remote_command: patch(m, "remote_command", st)?,
        transport: patch(m, "transport", st)?,
        environment: patch(m, "environment", st)?,
        os_icon: patch(m, "os_icon", st)?,
        tags: req(m, "tags", |v| strings_of(v, "tags"))?,
        notes: patch(m, "notes", st)?,
        favorite: req(m, "favorite", bool_of)?,
        secret_lookup: patch(m, "secret_lookup", st)?,
        has_password: req(m, "has_password", bool_of)?,
        group_uids: req(m, "group_uids", |v| {
            v.as_array()
                .ok_or_else(|| anyhow!("group_uids must be an array"))?
                .iter()
                .map(|x| uid_of(x, "group_uids"))
                .collect()
        })?,
        identity_uid: patch(m, "identity_uid", |v| uid_of(v, "identity_uid"))?,
    })
}

fn group_fields_of(m: &Map<String, Value>) -> Result<GroupFields> {
    let st = |v: &Value| str_of(v, "field");
    Ok(GroupFields {
        name: req(m, "name", st)?,
        default_identity_uid: patch(m, "default_identity_uid", |v| {
            uid_of(v, "default_identity_uid")
        })?,
        default_username: patch(m, "default_username", st)?,
        default_port: patch(m, "default_port", port_of)?,
        default_proxy_jump: patch(m, "default_proxy_jump", st)?,
        default_transport: patch(m, "default_transport", st)?,
        default_forward_agent: patch(m, "default_forward_agent", bool_of)?,
        parent_uid: patch(m, "parent_uid", |v| uid_of(v, "parent_uid"))?,
    })
}

fn identity_fields_of(m: &Map<String, Value>) -> Result<IdentityFields> {
    let st = |v: &Value| str_of(v, "field");
    Ok(IdentityFields {
        name: req(m, "name", st)?,
        username: patch(m, "username", st)?,
        has_password: req(m, "has_password", bool_of)?,
    })
}
