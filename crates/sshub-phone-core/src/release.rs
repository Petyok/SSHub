//! Share release, phone side: evaluate a challenge (everything the screen shows and every refusal is decided HERE from
//! the received bytes), then approve it. Port of `evaluateChallenge` / `approveRelease` of
//! `assets/companion/src/release.mjs` and the reply layout of `frames.mjs` (contract C16).
//!
//! Nothing here re-serialises a received op: the hash, the binding and the signature check all use the bytes as they
//! arrived. The chain counters of the requester's op are adopted only when its signature verified under a key this
//! phone enrolled with.

mod snapshot_issue;

use rand_core::RngCore;
use sshub_sync_proto::crypto::{aead_open, sas_words, verify, Argon2Profile, DeviceIdentity};
use sshub_sync_proto::limits::{CHALLENGE_EXPIRY_SECS, TIMESTAMP_SKEW_GRACE_SECS};
use sshub_sync_proto::oplog::canonical::body_name;
use sshub_sync_proto::oplog::{Op, OpBody};
use sshub_sync_proto::share_release::ReplyExt;
use sshub_sync_proto::wire::{
    binding_preimage, ChallengeWire, PhoneMaterial, ShareKind, ShareResponseWire,
};
use zeroize::Zeroizing;

use crate::chain::{self, Seen};
use crate::ctx::Ctx;
use crate::describe::{describe_op, is_device_id, parse_op};
use crate::error::{CoreError, Result};
use crate::identity::PhoneIdentity;
use crate::outgoing;
use crate::state::{COURIER_TAIL, HEAD_PENDING_MESSAGE};
use crate::text::safe_text;
use crate::views::{
    Check, IssuedCode, ManualEntry, Outgoing, RefusalView, ReleaseOptions, ShareRequestView,
    ShortCodeOffer,
};

/// The longest a signed request may precede its challenge, and the longest a challenge may live (120 s plus the skew
/// grace a terminal with a slow clock needs).
const MAX_AGE: i64 = CHALLENGE_EXPIRY_SECS + TIMESTAMP_SKEW_GRACE_SECS;

/// A share request that passed every check and waits for the person: what `approve` needs. The share is live in memory
/// until this is dropped (and zeroed then).
#[derive(Clone)]
pub struct PendingRelease {
    token: u64,
    ch: ChallengeWire,
    op: Op,
    host: [u8; 16],
    host_hex: String,
    requester: String,
    share: Zeroizing<[u8; 33]>,
    /// The requester's signature verified under a key this phone enrolled with.
    signature_verified: bool,
    /// What the phone signs to approve: `binding_preimage(...)`.
    binding_preimage: Vec<u8>,
    type_to_approve: Option<String>,
}

impl PendingRelease {
    pub fn token(&self) -> u64 {
        self.token
    }
    pub fn host_uid_hex(&self) -> &str {
        &self.host_hex
    }
    pub fn expires_at(&self) -> i64 {
        self.ch.expiry
    }
}

/// The key of a ring entry (hex), if it is well formed.
pub(crate) fn ring_pub(hex_text: &str) -> Option<[u8; 32]> {
    hex::decode(hex_text).ok()?.try_into().ok()
}

fn pass(text: impl Into<String>) -> Check {
    Check {
        text: text.into(),
        warn: false,
        ok: true,
    }
}

fn refuse(checks: &[Check], reason: impl Into<String>) -> RefusalView {
    // like the page: the failing rule is the last line of the list, marked bad
    let reason = reason.into();
    let mut checks = checks.to_vec();
    checks.push(Check {
        text: reason.clone(),
        warn: false,
        ok: false,
    });
    RefusalView {
        kind: crate::views::RefusalKind::Other,
        compared: Vec::new(),
        reason,
        checks,
    }
}

/// Opens the phone's own share. Hybrid (kind 0): `host_uid(16) || index || share` with a MANDATORY host comparison.
/// Envelope (kind 1): the storage key from the hybrid blob, then an AEAD envelope whose associated data names host_uid,
/// epoch and the holder's own index; the index is not known to the phone beforehand, so every index is tried and the
/// plaintext must name the index that opened it. (`unwrapPhoneShare` of the page.)
fn unwrap_phone_share(
    id: &mut PhoneIdentity,
    m: &PhoneMaterial,
    host: &[u8; 16],
) -> std::result::Result<Zeroizing<[u8; 33]>, String> {
    let mut out = Zeroizing::new([0u8; 33]);
    match m.kind {
        ShareKind::Hybrid => {
            let plain = Zeroizing::new(
                id.unwrap(&m.share_blob)
                    .map_err(|e| e.reason().to_string())?,
            );
            if plain.len() != 16 + 33 {
                return Err("wrapped share has the wrong length".to_string());
            }
            if plain[..16] != host[..] {
                return Err(
                    "the share inside the material belongs to a different credential".to_string(),
                );
            }
            if plain[16] == 0 {
                return Err("share index is zero".to_string());
            }
            out.copy_from_slice(&plain[16..]);
            Ok(out)
        }
        ShareKind::Envelope => {
            let key = Zeroizing::new(
                id.unwrap(&m.storage_blob)
                    .map_err(|e| e.reason().to_string())?,
            );
            let key: &[u8; 32] = key
                .as_slice()
                .try_into()
                .map_err(|_| "storage key has the wrong length".to_string())?;
            if m.share_blob.len() < 24 + 16 {
                return Err("envelope share is too short".to_string());
            }
            for index in 1..=255u8 {
                let mut aad = Vec::with_capacity(16 + 8 + 1);
                aad.extend_from_slice(host);
                aad.extend_from_slice(&m.epoch.to_be_bytes());
                aad.push(index);
                let Ok(plain) = aead_open(key, &aad, &m.share_blob) else {
                    continue;
                };
                let plain = Zeroizing::new(plain);
                if plain.len() != 33 || plain[0] != index {
                    return Err("envelope share names a different index".to_string());
                }
                out.copy_from_slice(&plain);
                return Ok(out);
            }
            Err("the envelope share does not belong to this credential or epoch".to_string())
        }
    }
}

/// `evaluateChallenge`: every rule, in the page's order, each refusal with the page's sentence. On success the view for
/// the screen and the pending request; a refusal is a value, never an error.
pub fn open_challenge(
    frame: &[u8],
    ctx: &mut Ctx,
    token: u64,
) -> std::result::Result<(ShareRequestView, PendingRelease), RefusalView> {
    let mut checks: Vec<Check> = Vec::new();
    let now = ctx.now;

    let ch = ChallengeWire::decode(frame)
        .map_err(|e| refuse(&checks, format!("challenge is malformed: {e:#}")))?;
    checks.push(pass("challenge frame is well formed"));

    let left = i128::from(ch.expiry) - i128::from(now);
    if left <= 0 {
        return Err(refuse(
            &checks,
            format!("challenge expired {} s ago", -left),
        ));
    }
    if left > i128::from(MAX_AGE) {
        return Err(refuse(
            &checks,
            format!("challenge claims to live {left} s, more than the {CHALLENGE_EXPIRY_SECS} s a terminal issues"),
        ));
    }
    checks.push(pass(format!("valid for another {left} s")));

    let op = parse_op(&ch.op_canonical, ch.op_signature)
        .map_err(|e| refuse(&checks, format!("operation is malformed: {e}")))?;
    let OpBody::ShareRelease {
        host_uid,
        request_id,
        requester,
        eph_x25519_pub,
    } = &op.body
    else {
        return Err(refuse(
            &checks,
            format!(
                "this is a {} operation, not a share release; this phone releases shares only",
                body_name(&op.body)
            ),
        ));
    };
    checks.push(pass("operation is a share release"));

    // The signed op is written right before the challenge is issued: an old signed request replayed inside a fresh
    // challenge (the signature covers the op alone) is refused, and a stale-looking one is flagged.
    let age = i128::from(now) - i128::from(op.timestamp);
    if age > i128::from(MAX_AGE) {
        return Err(refuse(
            &checks,
            format!(
                "the signed request is {} minutes old, but a request is signed right before its challenge: a terminal does not ask this way",
                age.div_euclid(60)
            ),
        ));
    }
    if age < -i128::from(TIMESTAMP_SKEW_GRACE_SECS) {
        return Err(refuse(&checks, "the signed request is dated in the future"));
    }
    let span = i128::from(ch.expiry) - i128::from(op.timestamp);
    if span < 0 || span > i128::from(MAX_AGE) {
        return Err(refuse(
            &checks,
            format!("the challenge expiry is {span} s from the signed request, outside what a terminal issues (0 to {MAX_AGE} s)"),
        ));
    }
    checks.push(pass(format!("the request was signed {} s ago", age.max(0))));

    // `decode_op` already refused a body that is not the four field form (host_uid, request_id, requester,
    // eph_x25519_pub); the requester id is a plain string there, so its shape is checked here.
    if !is_device_id(requester) {
        return Err(refuse(&checks, "share release fields are malformed"));
    }
    if *request_id != ch.request_id {
        return Err(refuse(
            &checks,
            "the operation asks for a different request id than the challenge carries",
        ));
    }
    if *requester != op.device_id {
        return Err(refuse(
            &checks,
            format!(
                "the operation is signed by {} but names {} as the requester",
                op.device_id, requester
            ),
        ));
    }
    // The signed op names the ephemeral key the share is sealed to: a challenge frame with another key than the signed
    // one would send the share to whoever swapped the frame.
    if *eph_x25519_pub != ch.eph_x25519_pub {
        return Err(refuse(
            &checks,
            "the signed operation names another ephemeral key than the challenge carries: the reply would not go to the device that asked",
        ));
    }
    checks.push(pass(
        "request id, requester and ephemeral key agree between the operation and the challenge",
    ));

    let known = ctx.state.ring.get(&op.device_id).cloned();
    let signature_verified = known.is_some();
    match &known {
        Some(k) => {
            let ok = ring_pub(&k.ed25519_pub)
                .is_some_and(|p| verify(&p, &ch.op_canonical, &ch.op_signature));
            if !ok {
                return Err(refuse(
                    &checks,
                    format!(
                        "the signature of {} does not verify over these operation bytes",
                        op.device_id
                    ),
                ));
            }
            checks.push(pass(format!(
                "signature of {} ({}) verifies",
                safe_text(&k.name),
                op.device_id
            )));
        }
        None => checks.push(Check {
            text: format!(
                "this phone has never been introduced to {}; its signature cannot be checked here",
                op.device_id
            ),
            warn: true,
            ok: true,
        }),
    }

    let share = unwrap_phone_share(&mut *ctx.id, &ch.phone_material, host_uid).map_err(|e| {
        refuse(
            &checks,
            format!("this phone cannot release for this request: {e}"),
        )
    })?;
    let kind = match ch.phone_material.kind {
        ShareKind::Hybrid => "hybrid",
        ShareKind::Envelope => "envelope",
    };
    checks.push(pass(format!(
        "own share unwrapped, credential id matches ({kind}, epoch {})",
        ch.phone_material.epoch
    )));

    let host_hex = hex::encode(host_uid);
    let bhash = ch.binding_hash();
    let preimage = binding_preimage(
        &ch.op_canonical,
        &ch.nonce,
        ch.expiry,
        &ch.eph_x25519_pub,
        ch.request_id,
        &ch.phone_material.hash(),
    );
    let op_view = describe_op(
        &op,
        &ch.op_canonical,
        &ctx.aliases(),
        Some(now),
        CHALLENGE_EXPIRY_SECS,
    );
    let type_to_approve = (!signature_verified).then(|| requester.clone());
    let short_code = match ctx.state.live_snapshot(&host_hex, now) {
        Some(s) => ShortCodeOffer::LiveUntil {
            expires_at: s.expires_at,
        },
        None => ShortCodeOffer::Available,
    };
    let plain = op_view.plain.clone();
    let view = ShareRequestView {
        token,
        op: op_view,
        host_uid: host_hex.clone(),
        requester: requester.clone(),
        binding_words: sas_words(&bhash),
        binding_hash: hex::encode(bhash),
        plain,
        checks,
        expires_at: ch.expiry,
        head_pending: ctx.state.head_pending,
        type_to_approve: type_to_approve.clone(),
        requester_name: ctx
            .aliases()
            .get(&op.device_id)
            .filter(|n| !n.is_empty())
            .map(|n| safe_text(n)),
        requester_known: known.is_some(),
        requester_key: known.as_ref().map(|k| k.ed25519_pub.clone()),
        share_kind: kind.to_string(),
        share_epoch: ch.phone_material.epoch,
        share_index: u32::from(share[0]),
        short_code,
    };
    let pending = PendingRelease {
        token,
        host: *host_uid,
        host_hex,
        requester: requester.clone(),
        share,
        signature_verified,
        binding_preimage: preimage,
        type_to_approve,
        op,
        ch,
    };
    Ok((view, pending))
}

/// `approveRelease` plus the decisions the page makes around it: expiry and the typed id are checked again at decision
/// time, the optional short code is sealed (one live per credential: ignored when one is live), the `HeadAttest` is
/// appended (counters adopted only from a verified requester op), the reply is built, sealed and framed.
pub fn approve(p: PendingRelease, opts: &ReleaseOptions, ctx: &mut Ctx) -> Result<Outgoing> {
    approve_with(p, opts, ctx, &snapshot_issue::PROFILES)
}

/// [`approve`] with the Argon2 profiles to try for the short code (tests pass a cheap one).
pub(crate) fn approve_with(
    p: PendingRelease,
    opts: &ReleaseOptions,
    ctx: &mut Ctx,
    profiles: &[Argon2Profile],
) -> Result<Outgoing> {
    if ctx.now >= p.ch.expiry {
        return Err(CoreError::refused(
            "The challenge expired: ask the terminal for a new request.",
        ));
    }
    // Before anything costly (the Argon2 short code) and before the typed id: a restored phone cannot sign at all.
    if ctx.state.head_pending {
        return Err(CoreError::refused(HEAD_PENDING_MESSAGE));
    }
    if let Some(want) = &p.type_to_approve {
        let typed = opts.typed_id.as_deref().unwrap_or("").trim().to_lowercase();
        if typed != *want {
            return Err(CoreError::bad(format!(
                "This phone has never met {want}: type that device id to approve."
            )));
        }
    }

    // One live short code per credential, enforced HERE (the phone), not by the peer.
    let issued = if opts.issue_short_code && ctx.state.live_snapshot(&p.host_hex, ctx.now).is_none()
    {
        Some(snapshot_issue::issue(
            &p.share,
            &p.host,
            ctx.now,
            &mut *ctx.rng,
            profiles,
        )?)
    } else {
        None
    };

    // lamport and epoch are adopted only from an op whose signature this phone verified.
    let seen = p.signature_verified.then_some(Seen {
        lamport: p.op.lamport,
        epoch: p.op.epoch,
    });
    chain::head_attest(&mut *ctx.state, &*ctx.id, ctx.now, seen)?;
    if let Some(i) = &issued {
        ctx.state.record_snapshot(&p.host_hex, i.record.clone());
    }

    let approval_sig = ctx.id.sign(&p.binding_preimage);
    let ext = ReplyExt {
        approval_sig,
        ops: chain::courier_ops(&*ctx.state, COURIER_TAIL),
        snapshot: issued.as_ref().map(|i| (p.host, i.blob.clone())),
    };
    let mut full_plain = Zeroizing::new(p.share.to_vec());
    full_plain.extend_from_slice(&ext.encode());
    let full = seal_reply(&p.ch, &full_plain, &mut *ctx.rng);
    let short = seal_reply(&p.ch, &p.share[..], &mut *ctx.rng);

    let note = format!(
        "Approved. Sent to {}. The terminal takes the first channel that arrives.",
        safe_text(&p.requester)
    );
    let mut out = outgoing::render(full, opts.qr, &note)?;
    out.audio_frame = opts.sound.then_some(short);
    out.manual = opts.by_hand.then(|| ManualEntry {
        index: u32::from(p.share[0]),
        words: sshub_sync_proto::crypto::bip39_encode(
            <&[u8; 32]>::try_from(&p.share[1..]).expect("32 bytes"),
        ),
    });
    out.short_code = issued.map(|i| IssuedCode {
        code: i.code,
        valid_until: i.record.expires_at,
        argon_mem_mib: i.record.mem_kib / 1024,
        argon_iters: i.record.iters,
        weak: i.record.weak,
    });
    Ok(out)
}

/// The plaintext sealed to the challenge's ephemeral key (sealed box, fresh randomness) and framed as kind 2.
fn seal_reply(ch: &ChallengeWire, plain: &[u8], rng: &mut dyn RngCore) -> Vec<u8> {
    let mut eph_secret = Zeroizing::new([0u8; 32]);
    let mut nonce = [0u8; 24];
    rng.fill_bytes(&mut eph_secret[..]);
    rng.fill_bytes(&mut nonce);
    let ciphertext = DeviceIdentity::seal_to_with(&ch.eph_x25519_pub, plain, &eph_secret, &nonce);
    ShareResponseWire {
        request_id: ch.request_id,
        ciphertext,
    }
    .encode()
}

#[cfg(test)]
mod tests;
