//! Kind 3 notices: a Red operation that needs this phone's co-signature, and a phone recovery this (still alive) phone
//! can cancel. Port of `recoveryNotice`, `redNotice` and `reenrollNotice` of `assets/companion/src/app.mjs`; what is
//! shown comes from the received op bytes, every refusal is a value.

use sshub_sync_proto::crypto::{chain_hash, verify};
use sshub_sync_proto::limits::{
    RECOVERY_TIMELOCK_SECS, RED_CEREMONY_TTL_SECS, TIMESTAMP_SKEW_GRACE_SECS,
};
use sshub_sync_proto::oplog::canonical::body_name;
use sshub_sync_proto::oplog::{DeviceRole, Op, OpBody};
use sshub_sync_proto::wire::RecoveryNoticeWire;

use crate::chain::{self, Seen};
use crate::ctx::Ctx;
use crate::describe::{describe_op, parse_op};
use crate::error::{CoreError, Result};
use crate::outgoing;
use crate::release::ring_pub;
use crate::state::COURIER_TAIL;
use crate::text::safe_text;
use crate::views::{Opened, Outgoing, RedNoticeView, ReenrollNoticeView, RefusalView};

/// A notice that passed every check and waits for the person.
#[derive(Clone)]
pub enum PendingNotice {
    Red {
        token: u64,
        op_hash: [u8; 32],
        op_timestamp: i64,
        /// Counters of the proposer's op, adopted only when its signature verified.
        seen: Option<Seen>,
        /// The proposer is this phone or a device it enrolled with; otherwise the id must be typed.
        met: bool,
        proposer: String,
        /// Why Approve is not offered, if so.
        blocked: Option<String>,
    },
    Reenroll {
        token: u64,
        op_hash: [u8; 32],
        seen: Option<Seen>,
    },
}

impl PendingNotice {
    pub fn token(&self) -> u64 {
        match self {
            Self::Red { token, .. } | Self::Reenroll { token, .. } => *token,
        }
    }
}

fn refused(reason: impl Into<String>) -> (Opened, Option<PendingNotice>) {
    (
        Opened::Refused {
            view: RefusalView {
                kind: crate::views::RefusalKind::Other,
                compared: Vec::new(),
                reason: reason.into(),
                checks: Vec::new(),
            },
        },
        None,
    )
}

/// A Red op needs the phone's proof. A mirror `DeviceAdd` and the revoke of a mirror are Yellow in the ring; the phone
/// cannot tell a holder from a mirror in a revoke, so it signs any `DeviceRevoke` the user approves (the proof is
/// inert). (`isRedOp` of ops.mjs.)
fn is_red_op(body: &OpBody) -> bool {
    match body {
        OpBody::DeviceAdd { role, .. } => *role == DeviceRole::Holder,
        OpBody::DeviceRevoke { .. }
        | OpBody::CertIssue { .. }
        | OpBody::ModeChange { .. }
        | OpBody::FactorRegister { .. }
        | OpBody::DeviceRoleChange { .. }
        | OpBody::BulkExport { .. }
        | OpBody::ExemptionSet { .. } => true,
        _ => false,
    }
}

/// `(proposer is a ring device or this phone, signature check)`: `Some(true)` verified, `Some(false)` does not verify,
/// `None` not checkable. The phone knows its own key too: a notice that claims this phone as the author must carry this
/// phone's signature (round 3, RP10).
fn check_signature(ctx: &Ctx, op: &Op, canonical: &[u8], sig: &[u8; 64]) -> (bool, Option<bool>) {
    let own = op.device_id == ctx.id.device_id();
    match ctx.state.ring.get(&op.device_id) {
        Some(k) => (
            true,
            Some(ring_pub(&k.ed25519_pub).is_some_and(|p| verify(&p, canonical, sig))),
        ),
        None if own => (
            true,
            Some(verify(&ctx.id.public().ed25519_pub, canonical, sig)),
        ),
        None => (false, None),
    }
}

fn seen_of(op: &Op, sig_ok: Option<bool>) -> Option<Seen> {
    (sig_ok == Some(true)).then_some(Seen {
        lamport: op.lamport,
        epoch: op.epoch,
    })
}

/// `recoveryNotice`: any kind 3 document. The view and the pending approval, or a refusal.
pub fn open_notice(frame: &[u8], ctx: &Ctx, token: u64) -> (Opened, Option<PendingNotice>) {
    let n = match RecoveryNoticeWire::decode(frame) {
        Ok(n) => n,
        Err(e) => return refused(format!("Notice is malformed: {e:#}")),
    };
    let op = match parse_op(&n.op_canonical, n.op_signature) {
        Ok(op) => op,
        Err(e) => return refused(format!("Notice is malformed: {e}")),
    };
    if matches!(op.body, OpBody::PhoneReenroll { .. }) {
        return open_reenroll(&n, &op, ctx, token);
    }
    if is_red_op(&op.body) {
        return open_red(&n, &op, ctx, token);
    }
    refused(format!(
        "This notice carries a {} operation, which needs nothing from this phone.",
        body_name(&op.body)
    ))
}

/// Age of a Red op in seconds (positive: in the past).
fn age(now: i64, op_timestamp: i64) -> i128 {
    i128::from(now) - i128::from(op_timestamp)
}

fn lapsed(age: i128) -> String {
    format!("This operation is {} days old; a Red ceremony lapses after 7 days. Ask the terminal to propose it again.", age.div_euclid(86_400))
}

fn open_red(
    n: &RecoveryNoticeWire,
    op: &Op,
    ctx: &Ctx,
    token: u64,
) -> (Opened, Option<PendingNotice>) {
    let aliases = ctx.aliases();
    let desc = describe_op(op, &n.op_canonical, &aliases, None, 0);
    let own = op.device_id == ctx.id.device_id();
    let (met, sig_ok) = check_signature(ctx, op, &n.op_canonical, &n.op_signature);
    if sig_ok == Some(false) {
        return refused(format!(
            "The signature of {} does not verify over this operation.",
            op.device_id
        ));
    }
    let a = age(ctx.now, op.timestamp);
    if a > i128::from(RED_CEREMONY_TTL_SECS) {
        return refused(lapsed(a));
    }
    if a < -i128::from(TIMESTAMP_SKEW_GRACE_SECS) {
        return refused("This operation is dated in the future.");
    }
    // A trust certificate is authored by the phone itself: the phone's own signature is the proof.
    let blocked = (matches!(op.body, OpBody::CertIssue { .. }) && !own).then(|| {
        "A trust certificate must be issued by the phone itself. This one was written by another device: do not approve it.".to_string()
    });
    let op_hash = chain_hash(&n.op_canonical);
    let who = aliases
        .get(&op.device_id)
        .filter(|s| !s.is_empty())
        .map_or("a device this phone has not met", String::as_str);
    let view = RedNoticeView {
        token,
        op: desc,
        op_hash: hex::encode(op_hash),
        banner: format!(
            "{} proposed by {} [{}]. It takes effect only with three proofs: this phone, a second attended device and a third factor. Your signature here is the phone's proof.",
            body_name(&op.body),
            safe_text(who),
            op.device_id
        ),
        signature_note: if sig_ok == Some(true) {
            "The signature of the proposer verifies.".to_string()
        } else {
            "This phone does not know the proposer, so its signature is not checked here.".to_string()
        },
        blocked: blocked.clone(),
        type_to_approve: (blocked.is_none() && !met).then(|| op.device_id.clone()),
        proposed_at: op.timestamp,
        lamport: op.lamport,
        seq: op.seq,
        lapses_at: op.timestamp.saturating_add(RED_CEREMONY_TTL_SECS),
        proposer_name: aliases.get(&op.device_id).filter(|s| !s.is_empty()).map(|s| safe_text(s)),
        proposer: op.device_id.clone(),
    };
    let pending = PendingNotice::Red {
        token,
        op_hash,
        op_timestamp: op.timestamp,
        seen: seen_of(op, sig_ok),
        met,
        proposer: op.device_id.clone(),
        blocked,
    };
    (Opened::RedNotice { view }, Some(pending))
}

fn open_reenroll(
    n: &RecoveryNoticeWire,
    op: &Op,
    ctx: &Ctx,
    token: u64,
) -> (Opened, Option<PendingNotice>) {
    let desc = describe_op(op, &n.op_canonical, &ctx.aliases(), None, 0);
    let (_, sig_ok) = check_signature(ctx, op, &n.op_canonical, &n.op_signature);
    if sig_ok == Some(false) {
        return refused(format!(
            "The signature of {} does not verify over this notice.",
            op.device_id
        ));
    }
    let OpBody::PhoneReenroll { initiated_at, .. } = &op.body else {
        return refused("This notice does not carry a phone recovery.");
    };
    let op_hash = chain_hash(&n.op_canonical);
    let view = ReenrollNoticeView {
        token,
        op: desc,
        op_hash: hex::encode(op_hash),
        ends_at: initiated_at.saturating_add(RECOVERY_TIMELOCK_SECS),
        proposed_at: op.timestamp,
        lamport: op.lamport,
        signature_note: if sig_ok == Some(true) {
            "The signature verifies.".to_string()
        } else {
            "This phone does not know the signing device, so the signature is not checked here."
                .to_string()
        },
    };
    (
        Opened::ReenrollNotice { view },
        Some(PendingNotice::Reenroll {
            token,
            op_hash,
            seen: seen_of(op, sig_ok),
        }),
    )
}

/// Signs the Red op (`RedPhoneSign`) or cancels the recovery (`PhoneReenrollCancel`) and returns the phone's newest ops
/// (the signed op included) for a terminal. The blocked case, the lapse and the typed id are decided again here.
pub fn approve_notice(p: PendingNotice, typed_id: Option<&str>, ctx: &mut Ctx) -> Result<Outgoing> {
    match p {
        PendingNotice::Red {
            op_hash,
            op_timestamp,
            seen,
            met,
            proposer,
            blocked,
            ..
        } => {
            if let Some(why) = blocked {
                return Err(CoreError::refused(why));
            }
            let a = age(ctx.now, op_timestamp);
            if a > i128::from(RED_CEREMONY_TTL_SECS) {
                return Err(CoreError::refused(lapsed(a)));
            }
            if !met && typed_id.unwrap_or("").trim().to_lowercase() != proposer {
                return Err(CoreError::bad(format!(
                    "This phone has never met {proposer}: type that device id to approve."
                )));
            }
            chain::red_phone_sign(&mut *ctx.state, &*ctx.id, op_hash, ctx.now, seen)?;
            outgoing::ops_out(
                &chain::courier_ops(&*ctx.state, COURIER_TAIL),
                "Signed. Carry this to a terminal: it holds the phone proof for the operation above. Show the QR code or paste the text.",
            )
        }
        PendingNotice::Reenroll { op_hash, seen, .. } => {
            chain::reenroll_cancel(&mut *ctx.state, &*ctx.id, op_hash, ctx.now, seen)?;
            outgoing::ops_out(
                &chain::courier_ops(&*ctx.state, COURIER_TAIL),
                "Cancellation signed. Show the QR code to any terminal, or paste the text into it. It carries the cancellation and the head of this phone chain.",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::PhoneIdentity;
    use crate::state::{PhoneState, RingDevice, HEAD_PENDING_MESSAGE};
    use rand_chacha::ChaCha8Rng;
    use rand_core::SeedableRng;
    use serde_json::Value;
    use sshub_sync_proto::crypto::DeviceIdentity;
    use sshub_sync_proto::oplog::canonical::decode_op;
    use sshub_sync_proto::oplog::CertScope;
    use sshub_sync_proto::wire::PhoneOpsWire;

    fn read(name: &str) -> Value {
        let p = format!(
            "{}/../../assets/companion/test/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    struct World {
        id: PhoneIdentity,
        state: PhoneState,
        rng: ChaCha8Rng,
        vectors: Value,
    }

    impl World {
        /// The phone of the fixtures (secret 42 x 32). `known` enrols the terminal that signed the op vectors.
        fn new(known: bool) -> Self {
            let vectors = read("ops.vectors.json");
            let mut w = Self {
                id: PhoneIdentity::new(&[0x42; 32], 0).unwrap(),
                state: PhoneState::default(),
                rng: ChaCha8Rng::seed_from_u64(3),
                vectors,
            };
            if known {
                let pubkey = w.vectors["meta"]["signer_ed25519_pub_hex"]
                    .as_str()
                    .unwrap()
                    .to_string();
                w.state.ring.insert(
                    SIGNER.to_string(),
                    RingDevice {
                        name: "terminal".into(),
                        ed25519_pub: pubkey,
                        enrolled_at: 0,
                    },
                );
            }
            w
        }
        fn ctx(&mut self, now: i64) -> Ctx<'_> {
            Ctx {
                id: &mut self.id,
                state: &mut self.state,
                rng: &mut self.rng,
                now,
            }
        }
        fn vector(&self, id: &str) -> (Vec<u8>, [u8; 64]) {
            let x = self.vectors["vectors"]
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
        fn open(&mut self, frame: &[u8], now: i64) -> (Opened, Option<PendingNotice>) {
            open_notice(frame, &self.ctx(now), 9)
        }
    }

    /// The device id of the key that signed the op vectors.
    const SIGNER: &str = "723ed85f0566df0c";
    /// The vectors are stamped at unix second 1.
    const AT: i64 = 100;

    fn notice(op: &(Vec<u8>, [u8; 64])) -> Vec<u8> {
        RecoveryNoticeWire {
            op_canonical: op.0.clone(),
            op_signature: op.1,
        }
        .encode()
    }

    fn red(o: Opened) -> RedNoticeView {
        match o {
            Opened::RedNotice { view } => view,
            other => panic!("not a red notice: {other:?}"),
        }
    }

    fn reason(o: &Opened) -> &str {
        match o {
            Opened::Refused { view } => &view.reason,
            other => panic!("not a refusal: {other:?}"),
        }
    }

    fn courier(out: &Outgoing) -> Vec<Op> {
        assert_eq!(out.kind, 5);
        PhoneOpsWire::decode(&out.frame)
            .unwrap()
            .ops
            .iter()
            .map(|(c, s)| decode_op(c, *s).unwrap())
            .collect()
    }

    #[test]
    fn a_red_op_from_a_known_proposer_is_shown_and_signed_with_the_phone_proof() {
        for id in [
            "device-add-holder",
            "device-role-change",
            "device-revoke",
            "mode-change-some",
        ] {
            let mut w = World::new(true);
            let v = w.vector(id);
            let (o, p) = w.open(&notice(&v), AT);
            let view = red(o);
            assert_eq!(
                view.signature_note, "The signature of the proposer verifies.",
                "{id}"
            );
            assert_eq!(view.type_to_approve, None, "{id}");
            assert_eq!(view.blocked, None, "{id}");
            assert_eq!(view.op_hash, hex::encode(chain_hash(&v.0)));
            assert!(
                view.banner
                    .contains(&format!("proposed by terminal [{SIGNER}]")),
                "{}",
                view.banner
            );
            assert!(view.op.danger_count > 0, "{id}");
            let out = approve_notice(p.unwrap(), None, &mut w.ctx(AT)).unwrap();
            let ops = courier(&out);
            assert_eq!(ops.len(), 1);
            let OpBody::RedPhoneSign { op_hash } = &ops[0].body else {
                panic!("{id}: not a RedPhoneSign")
            };
            assert_eq!(hex::encode(op_hash), view.op_hash);
            assert!(ops[0].verify(&w.id.public().ed25519_pub));
            assert_eq!(ops[0].device_id, w.id.device_id());
            // the verified proposer's counters were adopted: its lamport plus one
            let theirs = decode_op(&v.0, v.1).unwrap();
            assert_eq!(ops[0].lamport, theirs.lamport + 1, "{id}");
            assert_eq!(ops[0].seq, 1);
        }
    }

    #[test]
    fn a_mirror_a_non_red_op_and_garbage_need_nothing_from_the_phone() {
        let mut w = World::new(true);
        for id in [
            "device-add-mirror",
            "host-upsert-full",
            "head-attest-u64-max",
            "cert-extend",
        ] {
            let v = w.vector(id);
            let (o, p) = w.open(&notice(&v), AT);
            assert!(
                reason(&o).contains("needs nothing from this phone"),
                "{id}: {}",
                reason(&o)
            );
            assert!(p.is_none());
        }
        let (o, p) = w.open(&[1, 2, 3], AT);
        assert!(
            reason(&o).starts_with("Notice is malformed: "),
            "{}",
            reason(&o)
        );
        assert!(p.is_none());
        // an op that is not canonical is malformed too
        let v = w.vector("device-revoke");
        let spaced = String::from_utf8(v.0.clone())
            .unwrap()
            .replace("\"seq\":15,", "\"seq\": 15,");
        let (o, _) = w.open(&notice(&(spaced.into_bytes(), v.1)), AT);
        assert!(
            reason(&o).starts_with("Notice is malformed: "),
            "{}",
            reason(&o)
        );
    }

    #[test]
    fn su1_an_unmet_proposer_needs_its_id_typed_and_its_counters_are_not_adopted() {
        let mut w = World::new(false);
        let v = w.vector("device-add-holder");
        let frame = notice(&v);
        let (o, _) = w.open(&frame, AT);
        let view = red(o);
        assert_eq!(view.type_to_approve.as_deref(), Some(SIGNER));
        assert!(view.signature_note.contains("does not know the proposer"));
        assert!(view.banner.contains("a device this phone has not met"));
        for typed in [None, Some(""), Some("0000000000000000")] {
            let (_, p) = w.open(&frame, AT);
            let e = approve_notice(p.unwrap(), typed, &mut w.ctx(AT)).unwrap_err();
            assert!(matches!(e, CoreError::BadInput { .. }), "{typed:?}");
        }
        assert!(
            w.state.chain.is_empty(),
            "a refused approval writes nothing"
        );
        let (_, p) = w.open(&frame, AT);
        let out = approve_notice(p.unwrap(), Some(" 723ED85F0566DF0C "), &mut w.ctx(AT)).unwrap();
        assert_eq!(
            courier(&out)[0].lamport,
            1,
            "the unverified op moved nothing"
        );
    }

    #[test]
    fn a_known_proposer_with_a_bad_signature_is_refused() {
        let mut w = World::new(true);
        let mut v = w.vector("device-add-holder");
        v.1[0] ^= 1;
        let (o, p) = w.open(&notice(&v), AT);
        assert!(
            reason(&o).contains("does not verify over this operation"),
            "{}",
            reason(&o)
        );
        assert!(p.is_none());
    }

    #[test]
    fn red_ops_lapse_after_seven_days_and_a_future_date_is_refused() {
        let mut w = World::new(true);
        let v = w.vector("device-revoke"); // timestamp 1
        let frame = notice(&v);
        let ttl = RED_CEREMONY_TTL_SECS;
        assert!(
            matches!(w.open(&frame, 1 + ttl).0, Opened::RedNotice { .. }),
            "exactly 7 days is still inside"
        );
        let (o, p) = w.open(&frame, 2 + ttl);
        assert!(
            reason(&o).contains("7 days") && reason(&o).contains("7 days old"),
            "{}",
            reason(&o)
        );
        assert!(p.is_none());
        assert!(
            matches!(
                w.open(&frame, 1 - TIMESTAMP_SKEW_GRACE_SECS).0,
                Opened::RedNotice { .. }
            ),
            "inside the skew"
        );
        let (o, _) = w.open(&frame, -TIMESTAMP_SKEW_GRACE_SECS);
        assert_eq!(reason(&o), "This operation is dated in the future.");
        // and it is checked again when the person decides
        let (_, p) = w.open(&frame, AT);
        let e = approve_notice(p.unwrap(), None, &mut w.ctx(10 + ttl)).unwrap_err();
        assert!(
            matches!(e, CoreError::Refused { .. }) && e.reason().contains("days old"),
            "{e}"
        );
        assert!(w.state.chain.is_empty());
    }

    #[test]
    fn a_trust_certificate_written_by_another_device_is_blocked() {
        let mut w = World::new(true);
        let v = w.vector("cert-issue-hosts");
        let (o, p) = w.open(&notice(&v), AT);
        let view = red(o);
        assert!(view
            .blocked
            .as_deref()
            .unwrap()
            .contains("must be issued by the phone itself"));
        assert_eq!(view.type_to_approve, None, "no typed id can unlock it");
        let e = approve_notice(p.unwrap(), Some(SIGNER), &mut w.ctx(AT)).unwrap_err();
        assert!(matches!(e, CoreError::Refused { .. }));
        assert!(w.state.chain.is_empty());
    }

    /// An op authored by this very phone, as its chain would sign it.
    fn own_cert_issue(w: &World) -> (Vec<u8>, [u8; 64]) {
        let mut other = PhoneState::default();
        let a = chain::cert_issue(
            &mut other,
            &w.id,
            "0011223344556677".to_string(),
            CertScope::Hosts(vec![[1; 16]]),
            1_900_000_000,
            50,
            0,
            0,
            AT,
            None,
        )
        .unwrap();
        (a.canonical, a.signature)
    }

    #[test]
    fn rp10_a_notice_claiming_this_phone_as_author_must_carry_this_phones_signature() {
        let mut w = World::new(true);
        let genuine = own_cert_issue(&w);
        let (o, p) = w.open(&notice(&genuine), AT);
        let view = red(o);
        assert_eq!(
            view.signature_note,
            "The signature of the proposer verifies."
        );
        assert_eq!(
            view.blocked, None,
            "a CertIssue by the phone itself is the phone's own proof"
        );
        assert_eq!(
            view.type_to_approve, None,
            "this phone is not a stranger to itself"
        );
        approve_notice(p.unwrap(), None, &mut w.ctx(AT)).unwrap();

        let forged = (genuine.0.clone(), [7u8; 64]);
        let (o, p) = w.open(&notice(&forged), AT);
        assert!(reason(&o).contains("does not verify"), "{}", reason(&o));
        assert!(p.is_none(), "no Approve is offered");
        // a Red op claiming this phone, forged, from the vectors' own style
        let other = DeviceIdentity::from_seed(&[1; 32], "sshub/test/other");
        let mut op = decode_op(&genuine.0, genuine.1).unwrap();
        op.body = OpBody::DeviceRoleChange {
            device_id: "dddddddddddddddd".into(),
            to: DeviceRole::Holder,
        };
        let canon = op.canonical_bytes();
        let (o, _) = w.open(&notice(&(canon.clone(), other.sign(&canon))), AT);
        assert!(reason(&o).contains("does not verify"), "{}", reason(&o));
    }

    #[test]
    fn a_recovery_notice_is_cancelled_with_the_phones_own_signed_op() {
        let f = read("challenges.json");
        let frame = hex::decode(f["recovery_notice"]["frame_hex"].as_str().unwrap()).unwrap();
        let wire = RecoveryNoticeWire::decode(&frame).unwrap();
        let theirs = decode_op(&wire.op_canonical, wire.op_signature).unwrap();
        assert!(matches!(theirs.body, OpBody::PhoneReenroll { .. }));
        let mut w = World::new(true);
        let (o, p) = w.open(&frame, AT);
        let Opened::ReenrollNotice { view } = o else {
            panic!("not a reenroll notice")
        };
        assert_eq!(
            view.op_hash,
            f["recovery_notice"]["op_hash_hex"].as_str().unwrap()
        );
        assert_eq!(view.signature_note, "The signature verifies.");
        let OpBody::PhoneReenroll { initiated_at, .. } = &theirs.body else {
            unreachable!()
        };
        assert_eq!(view.ends_at, initiated_at + RECOVERY_TIMELOCK_SECS);
        assert!(view.op.danger_count > 0);
        let out = approve_notice(p.unwrap(), None, &mut w.ctx(AT)).unwrap();
        let ops = courier(&out);
        assert_eq!(ops.len(), 1);
        match &ops[0].body {
            OpBody::PhoneReenrollCancel {
                reenroll_hash,
                by_phone,
                third_factor,
            } => {
                assert_eq!(hex::encode(reenroll_hash), view.op_hash);
                assert!(*by_phone);
                assert!(third_factor.is_none());
            }
            other => panic!("not a cancellation: {other:?}"),
        }
        assert!(ops[0].verify(&w.id.public().ed25519_pub));
        assert!(ops[0].lamport > theirs.lamport);
        assert_eq!(ops[0].lamport, theirs.lamport + 1);
    }

    #[test]
    fn a_recovery_from_a_device_this_phone_never_met_can_still_be_cancelled_but_a_forged_one_is_refused(
    ) {
        let f = read("challenges.json");
        let frame = hex::decode(f["recovery_notice"]["frame_hex"].as_str().unwrap()).unwrap();
        let mut w = World::new(false);
        let (o, p) = w.open(&frame, AT);
        let Opened::ReenrollNotice { view } = o else {
            panic!("not a reenroll notice")
        };
        assert!(view
            .signature_note
            .contains("does not know the signing device"));
        let out = approve_notice(p.unwrap(), None, &mut w.ctx(AT)).unwrap();
        assert_eq!(
            courier(&out)[0].lamport,
            1,
            "an unverified op moves no counter"
        );

        // a known device whose signature does not hold
        let mut bad = frame.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        let mut w = World::new(true);
        let (o, p) = w.open(&bad, AT);
        assert!(
            reason(&o).contains("does not verify over this notice"),
            "{}",
            reason(&o)
        );
        assert!(p.is_none());
    }

    #[test]
    fn a_restored_phone_does_not_sign_a_notice_until_it_has_its_chain_head() {
        let mut w = World::new(true);
        w.state = PhoneState::restored();
        w.state.ring.insert(
            SIGNER.into(),
            RingDevice {
                name: "terminal".into(),
                ed25519_pub: w.vectors["meta"]["signer_ed25519_pub_hex"]
                    .as_str()
                    .unwrap()
                    .into(),
                enrolled_at: 0,
            },
        );
        let v = w.vector("device-revoke");
        let (_, p) = w.open(&notice(&v), AT);
        let e = approve_notice(p.unwrap(), None, &mut w.ctx(AT)).unwrap_err();
        assert!(matches!(e, CoreError::Refused { .. }));
        assert_eq!(e.reason(), HEAD_PENDING_MESSAGE);
        assert!(w.state.chain.is_empty());
    }
}
