//! What an approval screen shows for a signed op: a headline and one row per leaf of the body, built from the RECEIVED
//! canonical bytes (there is no description field anywhere). Port of `describeOp`, `flatten`, the DANGER tables and
//! `KNOWN_FIELDS` of `assets/companion/src/ops.mjs`.
//!
//! The bytes were already accepted by the strict `decode_op` (the encoder's exact output, so no duplicate key, escape
//! or whitespace can hide a field). `serde_json::Value` would sort the keys (no `preserve_order`), so the leaves are
//! walked with a tiny order-preserving reader: rows come in the canonical field order, like the page.

use std::collections::BTreeMap;

use sshub_sync_proto::oplog::canonical::{body_name, decode_op};
use sshub_sync_proto::oplog::{Op, OpBody};
use sshub_sync_proto::wire::MAX_COURIER_OP_BYTES;

use crate::text::{iso, safe_text};
use crate::views::{OpView, Row};

/// Fields of a host or group patch that C5 classes as dangerous.
const DANGEROUS_HOST: &[&str] = &[
    "address",
    "port",
    "username",
    "proxy_jump",
    "remote_command",
    "forward_agent",
    "transport",
    "environment",
    "secret_lookup",
    "has_password",
    "identity_uid",
    "group_uids",
];
const DANGEROUS_GROUP: &[&str] = &[
    "default_proxy_jump",
    "default_identity_uid",
    "default_username",
    "default_port",
    "default_transport",
    "default_forward_agent",
    "parent_uid",
];
/// Every field the Rust bodies know (op.rs HostFields, GroupFields) beyond the dangerous ones.
const SAFE_HOST: &[&str] = &["name", "label", "os_icon", "tags", "notes", "favorite"];
const SAFE_GROUP: &[&str] = &["name"];

enum Danger {
    /// Every leaf row is highlighted.
    All,
    /// A leaf is highlighted when any segment of its path is in the set.
    Set(&'static [&'static str]),
    None,
}

/// kind -> which leaf rows are dangerous. A kind that is not listed is highlighted entirely (a variant added to the
/// protocol later must not silently show as harmless).
fn danger_of(kind: &str) -> Danger {
    match kind {
        "HostUpsert" => Danger::Set(DANGEROUS_HOST),
        "GroupUpsert" => Danger::Set(DANGEROUS_GROUP),
        "ShareRelease" => Danger::Set(&["host_uid", "requester"]),
        // The ceremony proofs and checkpoints are inert (they apply nothing).
        "HeadAttest" | "RedApprove" | "RedPhoneSign" | "RedFactor" | "CheckpointPropose"
        | "CheckpointSign" => Danger::None,
        _ => Danger::All,
    }
}

/// `host_uid` etc. of a device id: 16 lower-case hex characters (the page's `HEX(16)` check; `decode_op` only requires
/// a string).
pub fn is_device_id(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// The page's `parseOp`: the strict proto decode plus the size cap and the header checks the page makes on top.
/// The error is a sentence for the screen.
pub fn parse_op(canonical: &[u8], signature: [u8; 64]) -> Result<Op, String> {
    if canonical.len() > MAX_COURIER_OP_BYTES {
        return Err(format!("op is larger than {MAX_COURIER_OP_BYTES} bytes"));
    }
    let op = decode_op(canonical, signature).map_err(|e| format!("{e:#}"))?;
    if !is_device_id(&op.device_id) {
        return Err("op device_id is not 16 hex characters".to_string());
    }
    Ok(op)
}

// ------------------------------------------------------------ order-preserving reader

#[derive(Clone, Debug)]
enum J {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

const MAX_DEPTH: usize = 64;

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> Option<()> {
        (self.peek() == Some(c)).then(|| self.i += 1)
    }

    fn word(&mut self, w: &str) -> Option<()> {
        if self.b[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Some(())
        } else {
            None
        }
    }

    fn value(&mut self, depth: usize) -> Option<J> {
        if depth > MAX_DEPTH {
            return None;
        }
        match self.peek()? {
            b'{' => {
                self.i += 1;
                let mut out = Vec::new();
                if self.eat(b'}').is_some() {
                    return Some(J::Obj(out));
                }
                loop {
                    let k = self.string()?;
                    self.eat(b':')?;
                    out.push((k, self.value(depth + 1)?));
                    if self.eat(b',').is_some() {
                        continue;
                    }
                    self.eat(b'}')?;
                    return Some(J::Obj(out));
                }
            }
            b'[' => {
                self.i += 1;
                let mut out = Vec::new();
                if self.eat(b']').is_some() {
                    return Some(J::Arr(out));
                }
                loop {
                    out.push(self.value(depth + 1)?);
                    if self.eat(b',').is_some() {
                        continue;
                    }
                    self.eat(b']')?;
                    return Some(J::Arr(out));
                }
            }
            b'"' => Some(J::Str(self.string()?)),
            b'n' => self.word("null").map(|_| J::Null),
            b't' => self.word("true").map(|_| J::Bool(true)),
            b'f' => self.word("false").map(|_| J::Bool(false)),
            b'-' | b'0'..=b'9' => {
                let from = self.i;
                while matches!(
                    self.peek(),
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    self.i += 1;
                }
                Some(J::Num(
                    String::from_utf8_lossy(&self.b[from..self.i]).into_owned(),
                ))
            }
            _ => None,
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = self.b.get(self.i..self.i + 4)?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                b'"' => return Some(String::from_utf8_lossy(&out).into_owned()),
                b'\\' => {
                    let e = self.peek()?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi)
                                && self.b[self.i..].starts_with(b"\\u")
                            {
                                self.i += 2;
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF)
                            } else {
                                hi
                            };
                            char::from_u32(cp).unwrap_or('\u{FFFD}')
                        }
                        _ => return None,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                _ => out.push(c),
            }
        }
    }
}

/// `{path, value}` of one leaf (`path` is empty for an empty top-level body).
struct Leaf {
    path: String,
    value: J,
}

/// Leaf names that hold opaque blobs (keys, sealed boxes, signatures). Only these are shortened, and only on rows that
/// are not marked danger: a person must never approve on a clipped value (an address of 60 characters, a host, an id).
const BLOB_FIELDS: &[&str] = &[
    "mlkem_pub",
    "ciphertext",
    "sealed",
    "sealed_share",
    "commitments",
    "sig",
    "paper_sig",
];

/// `^[A-Za-z0-9+/]{40,}={0,2}$`
fn looks_like_blob(s: &str) -> bool {
    let body = s.trim_end_matches('=');
    s.len() - body.len() <= 2
        && body.len() >= 40
        && body
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/')
}

fn fmt_leaf(v: &J, shorten: bool) -> String {
    match v {
        J::Null => "(cleared)".to_string(),
        J::Str(s) => {
            if shorten && looks_like_blob(s) {
                let head: String = s.chars().take(16).collect();
                format!(
                    "{} base64 chars: {}...",
                    s.chars().count(),
                    safe_text(&head)
                )
            } else {
                safe_text(s)
            }
        }
        J::Bool(b) => b.to_string(),
        J::Num(n) => n.clone(),
        J::Arr(_) | J::Obj(_) => String::new(),
    }
}

/// Every leaf becomes a row; nothing is dropped or summarised away except long base64 blobs in the fields of `BLOB_FIELDS`.
fn flatten(v: &J, path: &str, out: &mut Vec<Leaf>) {
    let join = |k: &str| {
        if path.is_empty() {
            k.to_string()
        } else {
            format!("{path}.{k}")
        }
    };
    match v {
        J::Obj(entries) => {
            if entries.is_empty() {
                out.push(Leaf {
                    path: path.to_string(),
                    value: J::Str("(none)".to_string()),
                });
            }
            for (k, x) in entries {
                flatten(x, &join(k), out);
            }
        }
        J::Arr(items) => {
            if items.is_empty() {
                out.push(Leaf {
                    path: path.to_string(),
                    value: J::Str("(empty list)".to_string()),
                });
            }
            for (i, x) in items.iter().enumerate() {
                flatten(x, &join(&i.to_string()), out);
            }
        }
        leaf => out.push(Leaf {
            path: path.to_string(),
            value: leaf.clone(),
        }),
    }
}

/// The fields object of the body of the received bytes (`{"Kind":{...}}`), in canonical order.
fn body_fields(canonical: &[u8]) -> Option<J> {
    let mut r = Reader { b: canonical, i: 0 };
    let J::Obj(root) = r.value(0)? else {
        return None;
    };
    let (_, body) = root.into_iter().find(|(k, _)| k == "body")?;
    let J::Obj(mut variants) = body else {
        return None;
    };
    (variants.len() == 1).then(|| variants.remove(0).1)
}

// ------------------------------------------------------------ describe

/// `name [id]`, or `(no name known) [id]`. A name from another machine is escaped.
pub fn who(aliases: &BTreeMap<String, String>, id: &str) -> String {
    let name = aliases.get(id).filter(|n| !n.is_empty());
    format!(
        "{}[{}]",
        name.map_or("(no name known) ".to_string(), |n| format!(
            "{} ",
            safe_text(n)
        )),
        safe_text(id)
    )
}

fn row(label: impl Into<String>, value: impl Into<String>, danger: bool) -> Row {
    Row {
        label: label.into(),
        value: value.into(),
        danger,
    }
}

/// `canonical` is the received bytes `op` was decoded from. `now` (the phone clock) flags a signed op much older than
/// its request: `stale_after` seconds.
pub fn describe_op(
    op: &Op,
    canonical: &[u8],
    aliases: &BTreeMap<String, String>,
    now: Option<i64>,
    stale_after: i64,
) -> OpView {
    let kind = body_name(&op.body).to_string();
    let mut rows: Vec<Row> = Vec::new();
    let headline;
    let requester;
    if let OpBody::ShareRelease {
        host_uid,
        request_id,
        requester: req,
        ..
    } = &op.body
    {
        let host = hex::encode(host_uid);
        requester = Some(req.clone());
        headline = format!(
            "Release your share of host credential {} to {}",
            safe_text(&host),
            who(aliases, req)
        );
        rows.push(row("Requesting device", who(aliases, req), true));
        rows.push(row("Host credential", safe_text(&host), true));
        rows.push(row("Request id", request_id.to_string(), false));
        rows.push(row(
            "Op signed by",
            who(aliases, &op.device_id),
            op.device_id != *req,
        ));
    } else {
        headline = format!("{kind} by {}", who(aliases, &op.device_id));
        requester = Some(op.device_id.clone());
        rows.push(row("Op signed by", who(aliases, &op.device_id), false));
        let danger = danger_of(&kind);
        let known: Option<(&[&str], &[&str])> = match kind.as_str() {
            "HostUpsert" => Some((DANGEROUS_HOST, SAFE_HOST)),
            "GroupUpsert" => Some((DANGEROUS_GROUP, SAFE_GROUP)),
            _ => None,
        };
        match body_fields(canonical) {
            Some(fields) => {
                let mut leaves = Vec::new();
                flatten(&fields, "", &mut leaves);
                for l in leaves {
                    let segs: Vec<&str> = l.path.split('.').collect();
                    let mut hot = match &danger {
                        Danger::All => true,
                        Danger::Set(set) => segs.iter().any(|s| set.iter().any(|d| d == s)),
                        Danger::None => false,
                    };
                    if let Some((dangerous, safe)) = known {
                        if segs[0] == "fields"
                            && segs.len() > 1
                            && !dangerous.contains(&segs[1])
                            && !safe.contains(&segs[1])
                        {
                            hot = true;
                        }
                    }
                    let label = if l.path.is_empty() {
                        kind.clone()
                    } else {
                        l.path.clone()
                    };
                    // a blob is clipped only in a field known to hold one and never on a dangerous row
                    let shorten = !hot && segs.iter().any(|s| BLOB_FIELDS.contains(s));
                    rows.push(row(safe_text(&label), fmt_leaf(&l.value, shorten), hot));
                }
            }
            // Unreachable after a strict decode; if it ever happens the person is told, never shown a clean screen.
            None => rows.push(row(
                "Body",
                "(the body could not be listed: do not approve)",
                true,
            )),
        }
    }
    let stale = now.is_some_and(|n| {
        i128::from(n).abs_diff(i128::from(op.timestamp)) > stale_after.max(0) as u128
    });
    rows.push(match now {
        Some(n) if stale => row(
            "Op time",
            format!(
                "{} ({} s from now)",
                iso(op.timestamp),
                i128::from(n).abs_diff(i128::from(op.timestamp))
            ),
            true,
        ),
        _ => row("Op time", iso(op.timestamp), false),
    });
    rows.push(row(
        "Op seq / epoch",
        format!("{} / {}", op.seq, op.epoch),
        false,
    ));
    let danger_count = rows.iter().filter(|r| r.danger).count() as u32;
    OpView {
        kind,
        headline,
        requester,
        rows,
        danger_count,
        hash: hex::encode(sshub_sync_proto::crypto::chain_hash(canonical)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const NOW: i64 = 1_900_000_000;

    fn vectors() -> serde_json::Value {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/companion/test/fixtures/ops.vectors.json"
        );
        serde_json::from_str(&std::fs::read_to_string(Path::new(p)).unwrap()).unwrap()
    }

    fn vector(v: &serde_json::Value, id: &str) -> (Vec<u8>, [u8; 64]) {
        let x = v["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == id)
            .unwrap();
        (
            hex::decode(x["canonical_hex"].as_str().unwrap()).unwrap(),
            hex::decode(x["signature_hex"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
        )
    }

    fn view(id: &str) -> OpView {
        let (c, s) = vector(&vectors(), id);
        let op = parse_op(&c, s).unwrap();
        describe_op(&op, &c, &BTreeMap::new(), None, 120)
    }

    fn meta(r: &Row) -> bool {
        matches!(
            r.label.as_str(),
            "Op signed by" | "Op time" | "Op seq / epoch"
        )
    }

    const INERT: &[&str] = &[
        "HeadAttest",
        "RedApprove",
        "RedPhoneSign",
        "RedFactor",
        "CheckpointPropose",
        "CheckpointSign",
    ];

    #[test]
    fn every_op_vector_parses_strictly_verifies_and_is_described_without_dropping_a_leaf() {
        let v = vectors();
        let signer: [u8; 32] = hex::decode(v["meta"]["signer_ed25519_pub_hex"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let all = v["vectors"].as_array().unwrap();
        assert!(all.len() >= 35);
        for x in all {
            let id = x["id"].as_str().unwrap();
            let (c, s) = vector(&v, id);
            let op = parse_op(&c, s).unwrap();
            assert!(
                sshub_sync_proto::crypto::verify(&signer, &c, &s),
                "{id}: signature"
            );
            let d = describe_op(&op, &c, &BTreeMap::new(), None, 120);
            assert_eq!(
                Some(d.kind.as_str()),
                x["op"]["body"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .next()
                    .map(String::as_str),
                "{id}"
            );
            assert!(d.rows.len() >= 3, "{id}");
            assert_eq!(
                d.danger_count as usize,
                d.rows.iter().filter(|r| r.danger).count()
            );
            if !INERT.contains(&d.kind.as_str())
                && d.kind != "ShareRelease"
                && !d.kind.ends_with("Upsert")
            {
                assert!(
                    d.rows.iter().filter(|r| !meta(r)).all(|r| r.danger),
                    "{id}: every row of a dangerous kind is highlighted"
                );
            }
            if INERT.contains(&d.kind.as_str()) {
                assert!(
                    d.rows.iter().filter(|r| !meta(r)).all(|r| !r.danger),
                    "{id}: inert kinds are not highlighted"
                );
            }
        }
    }

    #[test]
    fn host_upsert_highlights_exactly_the_dangerous_fields_of_c5() {
        let d = view("host-upsert-full");
        let danger: BTreeMap<_, _> = d
            .rows
            .iter()
            .map(|r| (r.label.as_str(), r.danger))
            .collect();
        for f in [
            "fields.address",
            "fields.port",
            "fields.username",
            "fields.proxy_jump",
        ] {
            assert!(danger[f], "{f}");
        }
        for f in ["fields.name", "fields.label"] {
            assert!(!danger[f], "{f}");
        }
        // canonical field order: host_uid before the patch, the patch in the encoder's order
        let labels: Vec<_> = d.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels[1], "host_uid");
        assert!(
            labels.iter().position(|l| *l == "fields.name")
                < labels.iter().position(|l| *l == "fields.address")
        );
        assert_eq!(*labels.last().unwrap(), "Op seq / epoch");
    }

    #[test]
    fn a_long_alphanumeric_address_is_never_clipped() {
        let v = vectors();
        let (c, s) = vector(&v, "host-upsert-full");
        let text = String::from_utf8(c).unwrap();
        let long = "a1B2".repeat(15); // 60 characters, base64-looking
        let c = text.replace("web1.example", &long).into_bytes();
        let op = parse_op(&c, s).unwrap();
        let d = describe_op(&op, &c, &BTreeMap::new(), None, 120);
        let r = d.rows.iter().find(|r| r.label == "fields.address").unwrap();
        assert_eq!(r.value, long);
        assert!(r.danger);
        // a long name is a safe field but not a blob field: also in full
        let name = "N0".repeat(30);
        let c = text.replace("web-1", &name).into_bytes();
        let op = parse_op(&c, s).unwrap();
        let d = describe_op(&op, &c, &BTreeMap::new(), None, 120);
        assert_eq!(
            d.rows
                .iter()
                .find(|r| r.label == "fields.name")
                .unwrap()
                .value,
            name
        );
    }

    #[test]
    fn cleared_empty_and_blob_leaves_are_shown_as_the_page_shows_them() {
        let d = view("host-upsert-every-field-cleared");
        assert!(d.rows.iter().any(|r| r.value == "(cleared)"));
        let d = view("host-upsert-empty-patch");
        let r = d.rows.iter().find(|r| r.label == "fields").unwrap();
        assert_eq!((r.value.as_str(), r.danger), ("(none)", false));
        // a blob in a blob field of a row that is not dangerous is clipped
        let d = view("red-factor");
        let sig = d.rows.iter().find(|r| r.label == "sig").unwrap();
        assert!(!sig.danger);
        assert!(
            sig.value.contains("base64 chars: ") && sig.value.ends_with("..."),
            "{}",
            sig.value
        );
        // a dangerous row is never clipped, a blob field of a dangerous kind included
        let d = view("device-add-holder");
        let key = d
            .rows
            .iter()
            .find(|r| r.label == "device.mlkem_pub")
            .unwrap();
        assert!(
            key.danger && !key.value.contains("base64 chars"),
            "{}",
            key.value.len()
        );
        assert_eq!(key.value.len(), 1580);
        // a hash is a value to compare, not a blob: shown in full
        let d = view("red-approve");
        let h = d.rows.iter().find(|r| r.label == "op_hash").unwrap();
        assert_eq!(h.value.len(), 64);
        let d = view("share-reshare-batch-empty");
        assert!(d.rows.iter().any(|r| r.value == "(empty list)"));
    }

    #[test]
    fn cert_extend_and_a_holder_admission_are_highlighted() {
        let d = view("cert-extend");
        let leaves: Vec<_> = d.rows.iter().filter(|r| !meta(r)).collect();
        assert_eq!(leaves.len(), 2);
        assert!(leaves.iter().all(|r| r.danger));
        assert!(view("device-add-holder").danger_count > 0);
        assert_eq!(view("head-attest-u64-max").danger_count, 0);
    }

    #[test]
    fn names_and_times_are_escaped_and_a_stale_op_is_flagged() {
        let (c, s) = vector(&vectors(), "device-revoke");
        let op = parse_op(&c, s).unwrap();
        let mut aliases = BTreeMap::new();
        aliases.insert(op.device_id.clone(), "evil\u{202e}name".to_string());
        let d = describe_op(&op, &c, &aliases, Some(NOW), 120);
        assert!(
            d.headline.contains("evil\\u202ename [723ed85f0566df0c]"),
            "{}",
            d.headline
        );
        assert_eq!(d.headline.matches('\u{202e}').count(), 0);
        let t = d.rows.iter().find(|r| r.label == "Op time").unwrap();
        assert!(t.danger && t.value.contains(" s from now)"), "{}", t.value);
        let fresh = describe_op(&op, &c, &BTreeMap::new(), Some(op.timestamp + 5), 120);
        assert!(
            !fresh
                .rows
                .iter()
                .find(|r| r.label == "Op time")
                .unwrap()
                .danger
        );
        assert!(fresh
            .headline
            .contains("(no name known) [723ed85f0566df0c]"));
    }

    #[test]
    fn hostile_or_non_canonical_bytes_are_refused() {
        let (c, s) = vector(&vectors(), "share-release-u64-max");
        let good = String::from_utf8(c.clone()).unwrap();
        assert!(parse_op(good.as_bytes(), s).is_ok());
        for bad in [
            good.replace("\"seq\":16", "\"seq\": 16"),
            good.replace(
                "\"requester\":\"723ed85f0566df0c\"",
                "\"requester\":\"723ed85f0566df0c\",\"requester\":\"0000000000000000\"",
            ),
            good.replace("\"lamport\":16,\"prev_hash\"", "\"prev_hash\""),
            good.replace("ShareRelease", "Mystery"),
        ] {
            assert!(parse_op(bad.as_bytes(), s).is_err(), "{bad}");
        }
        assert!(parse_op(&[0xff, 0xfe], s).is_err());
        assert!(parse_op(&vec![b' '; MAX_COURIER_OP_BYTES + 1], s)
            .unwrap_err()
            .contains("larger than"));
        assert!(
            !is_device_id("723ED85F0566DF0C")
                && !is_device_id("723ed85f0566df0")
                && is_device_id("723ed85f0566df0c")
        );
    }

    #[test]
    fn the_reader_keeps_order_and_unescapes() {
        let mut r = Reader {
            b: br#"{"b":[1,{"z":null,"a":"x\"\u00e9\ud83d\ude00"}],"a":true}"#,
            i: 0,
        };
        let J::Obj(o) = r.value(0).unwrap() else {
            panic!()
        };
        assert_eq!(
            o.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        let mut leaves = Vec::new();
        flatten(&J::Obj(o), "", &mut leaves);
        let shown: Vec<(String, String)> = leaves
            .iter()
            .map(|l| (l.path.clone(), fmt_leaf(&l.value, false)))
            .collect();
        let got: Vec<(&str, &str)> = shown
            .iter()
            .map(|(p, v)| (p.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("b.0", "1"),
                ("b.1.z", "(cleared)"),
                ("b.1.a", "x\"\u{e9}\u{1f600}"),
                ("a", "true")
            ]
        );
        assert!(Reader {
            b: b"{\"a\":",
            i: 0
        }
        .value(0)
        .is_none());
    }
}
