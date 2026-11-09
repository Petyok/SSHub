//! The uniffi surface: one stateful object, no global state. Kotlin passes bytes and `now` in; this returns views and
//! bytes. Every flow is `open_frame` (a view plus a token) then `decide(token, ...)` (the bytes to show or send).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use rand_core::{OsRng, RngCore};
use sshub_sync_proto::wire::{
    PhoneOpsWire, SignedOpBytes, TerminalsBlock, KIND_CERT_EXTEND_REQUEST, KIND_CERT_RENEW_REQUEST,
    KIND_CERT_REQUEST, KIND_CHALLENGE, KIND_ENROLL_COMMIT, KIND_ENROLL_CONFIRM, KIND_ENROLL_REVEAL,
    KIND_PHONE_OPS, KIND_RECOVERY_NOTICE, WIRE_MAGIC,
};
use zeroize::Zeroizing;

use crate::ble::BleSession;
use crate::cert::{self, PendingCert};
use crate::chain;
use crate::channels::{self, Scanner};
use crate::ctx::Ctx;
use crate::enroll::{self, EnrollFlow};
use crate::error::{CoreError, Result};
use crate::identity::{secret_from_words, PhoneIdentity};
use crate::notices::{self, PendingNotice};
use crate::release::{self, PendingRelease};
use crate::reshare::{self, PendingReshare};
use crate::state::{PhoneState, COURIER_TAIL};
use crate::text::{from_base64, safe_text, to_base64};
use crate::views::*;

enum Pending {
    Release(Box<PendingRelease>),
    Notice(PendingNotice),
    Cert(PendingCert),
    Reshare(PendingReshare),
    /// Head adoption: bound to the (seq, hash) of the head the person was shown.
    Chain {
        seq: u64,
        hash: Vec<u8>,
    },
}

struct Unlocked {
    id: PhoneIdentity,
    state: PhoneState,
    state_lost: bool,
}

struct Inner {
    /// The sealed state as stored; opened at `unlock` with the secret. `lock` puts the live state back here, so a
    /// lock followed by an unlock in the same process opens it (never starts empty).
    sealed: Option<Vec<u8>>,
    /// An unlock succeeded in this process: from now on missing sealed bytes are an error, not a fresh install.
    had_state: bool,
    /// `restore_from_words` ran: BLAKE3 of the restored secret. Only an `unlock` with that very secret starts from a
    /// restored (head pending) state; an unlock with any other secret (the old key after a cancelled prompt) opens the
    /// sealed state normally and clears this.
    restored: Option<[u8; 32]>,
    unlocked: Option<Unlocked>,
    rng: Box<dyn RngCore + Send>,
    next_token: u64,
    pending: HashMap<u64, Pending>,
    enroll: Option<EnrollFlow>,
    scanner: Scanner,
}

#[derive(uniffi::Object)]
pub struct PhoneCore {
    inner: Mutex<Inner>,
}

/// 32 CSPRNG bytes for a new phone: Kotlin wraps them with the Keystore key at once.
#[uniffi::export]
pub fn create_secret() -> Vec<u8> {
    let mut s = vec![0u8; 32];
    OsRng.fill_bytes(&mut s);
    s
}

/// The problem with a name the person typed for this phone (not empty, at most 64 bytes, no control characters), or None
/// when it is fine: show it before the scan, the same rules `Decision::EnrollName` applies.
#[uniffi::export]
pub fn check_enroll_name(name: String) -> Option<String> {
    enroll::check_name(&name)
        .err()
        .map(|e| e.reason().to_string())
}

/// Whether `word` (any case) is a word of the BIP39 list the 24 backup words and the manual entry use: input
/// validation while the person types (the four enrollment SAS words come from the same list). Not a check of the phrase (the checksum is checked by `restore_from_words`).
#[uniffi::export]
pub fn is_word(word: String) -> bool {
    sshub_sync_proto::crypto::is_bip39_word(&word)
}

/// An SSHub frame: the `SWR1` magic and a kind from 1 to 10 (what `PhoneCore::open_frame` can look at).
fn is_sshub_frame(b: &[u8]) -> bool {
    kind_of(b).is_some_and(|k| (1..=12).contains(&k))
}
fn kind_of(frame: &[u8]) -> Option<u8> {
    (frame.len() >= 7 && frame[..4] == WIRE_MAGIC).then(|| frame[6])
}

fn refused(reason: impl Into<String>) -> Opened {
    Opened::Refused {
        view: RefusalView {
            kind: crate::views::RefusalKind::Other,
            compared: Vec::new(),
            reason: reason.into(),
            checks: Vec::new(),
        },
    }
}

fn info(u: &Unlocked, now: i64) -> UnlockInfo {
    UnlockInfo {
        device_id: u.id.device_id().to_string(),
        enrolled: u.state.enrolled.is_some(),
        head_pending: u.state.head_pending,
        state_lost: u.state_lost,
        head_seq: u.state.head().map(|e| e.seq),
        wrap_epoch: u.state.wrap_epoch,
        live_short_codes: u
            .state
            .snapshots
            .keys()
            .filter(|k| u.state.live_snapshot(k, now).is_some())
            .count() as u32,
        device_name: u.state.enrolled.as_ref().map(|e| safe_text(&e.name)),
        ring_epoch: u.state.epoch,
        ring: u
            .state
            .ring
            .iter()
            .map(|(id, d)| RingEntry {
                device_id: id.clone(),
                name: d.name.clone(),
            })
            .collect(),
    }
}

impl Inner {
    /// Ends the unlocked session and keeps its state, sealed under its own secret (the bytes `sealed_state()` gives), in
    /// `sealed` for the next `unlock`. Nothing is dropped without being sealed first.
    fn seal_and_drop(&mut self) {
        if let Some(mut u) = self.unlocked.take() {
            self.sealed = Some(u.state.seal(u.id.secret(), &mut *self.rng));
        }
    }
}

impl PhoneCore {
    fn lock_inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn build(sealed: Option<Vec<u8>>, rng: Box<dyn RngCore + Send>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                sealed,
                had_state: false,
                restored: None,
                unlocked: None,
                rng,
                next_token: 1,
                pending: HashMap::new(),
                enroll: None,
                scanner: Scanner::new(),
            }),
        })
    }

    /// Tests inject a seeded RNG. Not part of the exported surface.
    #[cfg(any(test, feature = "testing"))]
    pub fn with_rng(sealed_state: Option<Vec<u8>>, rng: Box<dyn RngCore + Send>) -> Arc<Self> {
        Self::build(sealed_state, rng)
    }
}

impl Inner {
    fn token(&mut self) -> u64 {
        let t = self.next_token;
        self.next_token += 1;
        t
    }

    /// Splits the borrow: the unlocked identity and state, the RNG and the clock as the flows' context.
    fn ctx(&mut self, now: i64) -> Result<Ctx<'_>> {
        let u = self.unlocked.as_mut().ok_or(CoreError::Locked)?;
        Ok(Ctx {
            id: &mut u.id,
            state: &mut u.state,
            rng: &mut *self.rng,
            now,
        })
    }

    fn open_chain(
        &mut self,
        ops: &[SignedOpBytes],
        sender: Option<[u8; 8]>,
        terminals: Option<TerminalsBlock>,
    ) -> Result<Opened> {
        let t = self.token();
        let u = self.unlocked.as_mut().ok_or(CoreError::Locked)?;
        if !u.state.head_pending {
            return Ok(refused("This phone already knows its own chain; a list of its own ops needs nothing from it."));
        }
        let r = match chain::adopt_own_chain(&mut u.state, &u.id, ops, sender, terminals) {
            Ok(r) => r,
            Err(e) => return Ok(refused(e.reason().to_string())),
        };
        if r.agreed {
            self.pending.insert(
                t,
                Pending::Chain {
                    seq: r.head_seq,
                    hash: r.head_hash.clone(),
                },
            );
        }
        Ok(Opened::Chain {
            view: ChainAdopted {
                token: t,
                agreed: r.agreed,
                head_seq: r.head_seq,
                newest_at: r.newest_at,
                ops: r.ops,
                previous_seq: r.previous_seq,
                single_source: r.single_source,
                terminals_count: r.terminals.map(|t| u32::from(t.count)),
                terminals_newest_at: r.terminals.map(|t| t.newest_op_ts),
                sender: r.sender.clone(),
                summary: format!(
                    "Newest op in this list: seq {}, signed {}Z ({} op(s) read).",
                    r.head_seq,
                    crate::text::iso(r.newest_at).chars().take(16).collect::<String>(),
                    r.ops
                ),
                note: match (r.agreed, r.previous_seq) {
                    (true, _) if r.single_source => chain::single_source_note(r.head_seq, r.terminals),
                    (true, _) => format!("This phone will continue after seq {}. If any terminal holds a higher seq of this phone, do not adopt this: signing on top of an old head forks the chain. Take a newer list instead.", r.head_seq),
                    (false, Some(p)) => format!("The earlier list ended at seq {p}, so one of the two is behind. Take a list from another terminal that is up to date."),
                    (false, None) if r.sender.is_none() => "This list does not say which terminal made it, so it cannot be one of the two. Take the chain from two different terminals that name themselves.".to_string(),
                    (false, None) => "Take a second list from another terminal before this chain head is trusted.".to_string(),
                },
            },
        })
    }

    fn open_frame(&mut self, frame: &[u8], now: i64) -> Result<Opened> {
        if self.unlocked.is_none() {
            return Err(CoreError::Locked);
        }
        let Some(kind) = kind_of(frame) else {
            return Ok(refused(
                "Not a SSHub document: it does not start with the frame magic.",
            ));
        };
        if frame.len() > sshub_sync_proto::limits::MAX_FRAME_BYTES {
            return Ok(refused("This document is larger than any SSHub frame."));
        }
        // The pending requests are replaced only when a frame has parsed as a request that replaces them (below): a
        // garbage, truncated or refused frame, or a frame that is not a request at all, leaves the queue alone.
        let t = self.token();
        match kind {
            KIND_CHALLENGE => {
                let mut ctx = self.ctx(now)?;
                match release::open_challenge(frame, &mut ctx, t) {
                    Ok((view, p)) => {
                        self.pending.clear();
                        self.pending.insert(t, Pending::Release(Box::new(p)));
                        Ok(Opened::ShareRequest { view })
                    }
                    Err(view) => Ok(Opened::Refused { view }),
                }
            }
            KIND_RECOVERY_NOTICE => {
                let ctx = self.ctx(now)?;
                let (o, p) = notices::open_notice(frame, &ctx, t);
                if let Some(p) = p {
                    self.pending.clear();
                    self.pending.insert(t, Pending::Notice(p));
                }
                Ok(o)
            }
            KIND_CERT_REQUEST | KIND_CERT_EXTEND_REQUEST | KIND_CERT_RENEW_REQUEST => {
                let ctx = self.ctx(now)?;
                let (o, p) = if kind == KIND_CERT_REQUEST {
                    cert::open_cert_request(frame, &ctx, t)
                } else if kind == KIND_CERT_RENEW_REQUEST {
                    cert::open_cert_renew(frame, &ctx, t)
                } else {
                    cert::open_cert_extend(frame, &ctx, t)
                };
                if let Some(p) = p {
                    self.pending.clear();
                    self.pending.insert(t, Pending::Cert(p));
                }
                Ok(o)
            }
            KIND_PHONE_OPS => {
                let (ops, sender, terminals) = match PhoneOpsWire::decode(frame) {
                    Ok(w) => (w.ops, w.sender, w.terminals),
                    Err(e) => return Ok(refused(format!("Not a list of ops: {e:#}"))),
                };
                let own = self
                    .unlocked
                    .as_ref()
                    .map(|u| u.id.device_id().to_string())
                    .unwrap_or_default();
                if ops
                    .iter()
                    .all(|(c, _)| op_author(c).as_deref() == Some(own.as_str()))
                    && !ops.is_empty()
                {
                    return self.open_chain(&ops, sender, terminals);
                }
                let mut ctx = self.ctx(now)?;
                let (o, p) = reshare::open_reshare(ops, &mut ctx, t);
                if let Some(p) = p {
                    self.pending.clear();
                    self.pending.insert(t, Pending::Reshare(p));
                }
                Ok(o)
            }
            KIND_ENROLL_COMMIT | KIND_ENROLL_REVEAL | KIND_ENROLL_CONFIRM => {
                let flow = self.enroll.get_or_insert_with(EnrollFlow::new);
                let u = self.unlocked.as_mut().ok_or(CoreError::Locked)?;
                let mut ctx = Ctx {
                    id: &mut u.id,
                    state: &mut u.state,
                    rng: &mut *self.rng,
                    now,
                };
                let o = flow.on_frame(frame, &mut ctx, t);
                // The flow ends when the enrollment is done or a refusal stopped it (the terminal's reveal does not open its
                // commitment, it declined, a bad signature, unusable keys). A frame at the wrong step or a stray document
                // is refused and the enrollment goes on where it was: one wrong QR must not kill the session.
                let ended = match &o {
                    Opened::Enrolled { .. } => true,
                    Opened::Refused { view } => {
                        !matches!(view.kind, RefusalKind::WrongOrder | RefusalKind::StrayFrame)
                    }
                    _ => false,
                };
                if ended {
                    self.enroll = None;
                }
                Ok(o)
            }
            other => Ok(refused(format!(
                "This document (kind {other}) is not something a phone answers."
            ))),
        }
    }
}

/// The device id an op names as its author, read from the received bytes without trusting anything else.
fn op_author(canonical: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(canonical).ok()?;
    v.get("device_id")?.as_str().map(str::to_string)
}

fn none_outgoing(note: &str) -> Outgoing {
    Outgoing {
        kind: 0,
        frame: Vec::new(),
        text: String::new(),
        qr: Vec::new(),
        qr_too_big: false,
        audio_frame: None,
        manual: None,
        short_code: None,
        note: note.to_string(),
    }
}

#[uniffi::export]
impl PhoneCore {
    /// `sealed_state` is the blob `sealed_state()` returned last time, or None on first start.
    #[uniffi::constructor]
    pub fn new(sealed_state: Option<Vec<u8>>) -> Arc<Self> {
        Self::build(sealed_state, Box::new(OsRng))
    }

    /// Opens the stored state with the secret and derives the identity. A state that does not open (damaged, or
    /// sealed under another key) starts as a restored phone: it must take its chain from a terminal.
    pub fn unlock(&self, secret: Vec<u8>) -> Result<UnlockInfo> {
        let s: Zeroizing<[u8; 32]> = Zeroizing::new(
            secret
                .try_into()
                .map_err(|_| CoreError::bad("the phone secret must be 32 bytes"))?,
        );
        let mut g = self.lock_inner();
        g.seal_and_drop();
        let restore_of_this_secret =
            g.restored.take() == Some(sshub_sync_proto::crypto::chain_hash(&s[..]));
        let (state, lost) = if restore_of_this_secret {
            (PhoneState::restored(), false)
        } else {
            match g.sealed.as_deref() {
                None if g.had_state => {
                    return Err(CoreError::refused(
                        "This phone was unlocked before, but no stored state is held any more: restart the app. Nothing was changed.",
                    ))
                }
                None => (PhoneState::default(), false),
                Some(b) => match PhoneState::open(b, &s) {
                    Ok(st) => (st, false),
                    Err(_) => (PhoneState::restored(), true),
                },
            }
        };
        let id = PhoneIdentity::new(&s, state.wrap_epoch)?;
        g.had_state = true;
        g.sealed = None;
        g.pending.clear();
        g.enroll = None;
        g.unlocked = Some(Unlocked {
            id,
            state,
            state_lost: lost,
        });
        Ok(info(g.unlocked.as_ref().unwrap(), 0))
    }

    /// The tokens of the requests that are pending now (a share request, a notice, a trust request or extension, a
    /// reshare, an agreed chain head), ascending. Call it after every `open_frame`, `take_chain`, `decide` and `lock`:
    /// `open_frame` replaces the pending requests when a new request parses, `decide` consumes one, and `lock` drops all,
    /// and this is exactly what Rust still holds.
    pub fn pending_tokens(&self) -> Vec<u64> {
        let g = self.lock_inner();
        let mut t: Vec<u64> = g.pending.keys().copied().collect();
        t.sort_unstable();
        t
    }

    /// Zeroizes the secret and every key.
    pub fn lock(&self) {
        let mut g = self.lock_inner();
        g.seal_and_drop();
        g.pending.clear();
        g.enroll = None;
    }

    pub fn status(&self, now: i64) -> Result<UnlockInfo> {
        let g = self.lock_inner();
        Ok(info(g.unlocked.as_ref().ok_or(CoreError::Locked)?, now))
    }

    pub fn backup_words(&self) -> Result<String> {
        let g = self.lock_inner();
        Ok(g.unlocked.as_ref().ok_or(CoreError::Locked)?.id.words())
    }

    /// The secret of 24 typed words; the next `unlock` with it starts as a restored phone (no signing until the chain
    /// head is taken from a terminal). Kotlin wraps the returned secret like a fresh one.
    pub fn restore_from_words(&self, words: String) -> Result<Vec<u8>> {
        let s = secret_from_words(&words)?;
        let mut g = self.lock_inner();
        g.restored = Some(sshub_sync_proto::crypto::chain_hash(&s[..]));
        g.seal_and_drop();
        Ok(s.to_vec())
    }

    /// A frame from any channel. Kinds 1 (share request), 3 (Red notice, recovery), 5 (own chain list or a reshare),
    /// 6 to 8 (enrollment), 9 and 10 (trust session).
    pub fn open_frame(&self, frame: Vec<u8>, now: i64) -> Result<Opened> {
        self.lock_inner().open_frame(&frame, now)
    }

    pub fn decide(&self, token: u64, decision: Decision, now: i64) -> Result<Outgoing> {
        let mut guard = self.lock_inner();
        let g = &mut *guard;
        // enrollment decisions belong to the flow, not to the pending map
        if let Decision::EnrollName { .. } | Decision::EnrollSas { .. } = decision {
            let u = g.unlocked.as_mut().ok_or(CoreError::Locked)?;
            let mut flow = g
                .enroll
                .take()
                .ok_or_else(|| CoreError::bad("no enrollment is running"))?;
            let mut ctx = Ctx {
                id: &mut u.id,
                state: &mut u.state,
                rng: &mut *g.rng,
                now,
            };
            let r = flow.decide(&decision, &mut ctx);
            let ended = matches!(&decision, Decision::EnrollSas { matches: false });
            // a refusal for what was typed leaves the enrollment where it was
            if !ended && (r.is_ok() || matches!(r, Err(CoreError::BadInput { .. }))) {
                g.enroll = Some(flow);
            }
            return r;
        }
        let p = g
            .pending
            .remove(&token)
            .ok_or_else(|| CoreError::bad("this request is no longer pending"))?;
        // an explicit cancel consumes the request
        if matches!(decision, Decision::Refuse) {
            return Ok(none_outgoing("Refused."));
        }
        // A decision that is refused for what the person typed (the device id) leaves the request open: only a completed
        // decision, a refusal that ends the request (expired, blocked) or an explicit cancel consumes the token.
        let keep = match &p {
            Pending::Release(r) => Some(Pending::Release(r.clone())),
            Pending::Notice(n) => Some(Pending::Notice(n.clone())),
            Pending::Cert(c) => Some(Pending::Cert(c.clone())),
            Pending::Chain { seq, hash } => Some(Pending::Chain {
                seq: *seq,
                hash: hash.clone(),
            }),
            _ => None,
        };
        // A restored phone cannot sign: refuse before any work and keep the request, so it can be approved once the head
        // is taken (the page leaves the request on screen the same way).
        if let (Some(Pending::Release(_)), Decision::Release { .. }) = (&keep, &decision) {
            if g.unlocked.as_ref().is_some_and(|u| u.state.head_pending) {
                g.pending.insert(token, p);
                return Err(CoreError::refused(crate::state::HEAD_PENDING_MESSAGE));
            }
        }
        let mut ctx = g.ctx(now)?;
        let out = ctx.atomically(|ctx| match (p, decision) {
            (Pending::Release(p), Decision::Release { options }) => {
                release::approve(*p, &options, ctx)
            }
            (Pending::Notice(p), Decision::Approve { typed_id }) => {
                notices::approve_notice(p, typed_id.as_deref(), ctx)
            }
            (Pending::Cert(p), Decision::Approve { typed_id }) => {
                cert::approve_cert(p, typed_id.as_deref(), ctx)
            }
            (Pending::Reshare(p), Decision::Approve { .. }) => reshare::publish_reshare(p, ctx),
            (
                Pending::Chain { seq: shown, hash },
                d @ (Decision::Approve { .. } | Decision::AcceptSingleSource),
            ) => {
                let ack = matches!(d, Decision::AcceptSingleSource);
                let (seq, n) = chain::accept_head(ctx.state, (shown, &hash), ack)?;
                Ok(none_outgoing(&format!(
                    "Chain taken: continuing after seq {seq} ({n} op(s) read)."
                )))
            }
            _ => Err(CoreError::bad("that decision does not fit this request")),
        });
        if matches!(out, Err(CoreError::BadInput { .. })) {
            if let Some(k) = keep {
                g.pending.insert(token, k);
            }
        }
        out
    }

    /// The head adoption step 1 for a list given directly (the same as `open_frame` on a kind 5 list of own ops).
    pub fn take_chain(&self, ops_frame: Vec<u8>) -> Result<ChainAdopted> {
        let mut g = self.lock_inner();
        let wire = PhoneOpsWire::decode(&ops_frame)
            .map_err(|e| CoreError::bad(format!("Not a list of ops: {e:#}")))?;
        match g.open_chain(&wire.ops, wire.sender, wire.terminals)? {
            Opened::Chain { view } => Ok(view),
            Opened::Refused { view } => Err(CoreError::bad(view.reason)),
            _ => Err(CoreError::internal("unexpected view")),
        }
    }

    /// "This key never signed anything": only with this phone's device id typed.
    pub fn never_signed(&self, typed_id: String) -> Result<()> {
        let mut g = self.lock_inner();
        let u = g.unlocked.as_mut().ok_or(CoreError::Locked)?;
        chain::confirm_no_chain(&mut u.state, &u.id, &typed_id)
    }

    /// The newest ops of this phone's chain as a kind 5 frame in base64, for a terminal without a camera.
    pub fn export_ops_text(&self) -> Result<String> {
        let g = self.lock_inner();
        let u = g.unlocked.as_ref().ok_or(CoreError::Locked)?;
        if u.state.chain.is_empty() {
            return Err(CoreError::bad("This phone has not written anything yet."));
        }
        Ok(to_base64(
            &PhoneOpsWire {
                sender: None,
                terminals: None,
                ops: chain::courier_ops(&u.state, COURIER_TAIL),
            }
            .encode(),
        ))
    }

    /// The newest ops of this phone's chain (what `export_ops_text` carries), one `OpView` per op in chain order,
    /// rendered from the signed bytes with the same rows and danger marks as an approval screen.
    pub fn export_ops_view(&self) -> Result<Vec<OpView>> {
        let g = self.lock_inner();
        let u = g.unlocked.as_ref().ok_or(CoreError::Locked)?;
        let mut aliases = u.state.aliases.clone();
        for (id, d) in &u.state.ring {
            aliases.insert(id.clone(), d.name.clone());
        }
        chain::courier_ops(&u.state, COURIER_TAIL)
            .iter()
            .map(|(canonical, sig)| {
                let op = sshub_sync_proto::oplog::canonical::decode_op(canonical, *sig).map_err(
                    |e| {
                        CoreError::internal(format!(
                            "an op of this phone's own chain does not parse: {e:#}"
                        ))
                    },
                )?;
                Ok(crate::describe::describe_op(
                    &op, canonical, &aliases, None, 0,
                ))
            })
            .collect()
    }

    /// The state encrypted under the secret. Call after every mutation; Kotlin writes it atomically.
    pub fn sealed_state(&self) -> Result<Vec<u8>> {
        let mut g = self.lock_inner();
        let g = &mut *g;
        let u = g.unlocked.as_mut().ok_or(CoreError::Locked)?;
        Ok(u.state.seal(u.id.secret(), &mut *g.rng))
    }

    pub fn rotate_wrap_keys(&self, now: i64) -> Result<Outgoing> {
        let mut g = self.lock_inner();
        let mut ctx = g.ctx(now)?;
        ctx.atomically(|c| reshare::rotate_wrap_keys(c))
    }

    pub fn short_codes(&self, now: i64) -> Result<Vec<ShortCodeEntry>> {
        let g = self.lock_inner();
        let u = g.unlocked.as_ref().ok_or(CoreError::Locked)?;
        Ok(u.state
            .snapshots
            .iter()
            .map(|(host, s)| ShortCodeEntry {
                host_uid: host.clone(),
                issued_at: s.issued_at,
                expires_at: s.expires_at,
                live: u.state.live_snapshot(host, now).is_some(),
                argon_mem_mib: s.mem_kib / 1024,
                argon_iters: s.iters,
                weak: s.weak,
            })
            .collect())
    }

    /// The person declares the old code dead (never delivered, or the credential was reissued).
    pub fn declare_short_code_dead(&self, host_uid: String) -> Result<()> {
        let mut g = self.lock_inner();
        g.unlocked
            .as_mut()
            .ok_or(CoreError::Locked)?
            .state
            .invalidate_snapshot(&host_uid);
        Ok(())
    }

    pub fn issued_certs(&self, now: i64) -> Result<Vec<IssuedCertView>> {
        let g = self.lock_inner();
        let u = g.unlocked.as_ref().ok_or(CoreError::Locked)?;
        let mut aliases = u.state.aliases.clone();
        for (id, d) in &u.state.ring {
            aliases.insert(id.clone(), d.name.clone());
        }
        Ok(cert::issued_certs(&u.state, &aliases, now))
    }

    pub fn revoke_cert(&self, cert_id: String, now: i64) -> Result<Outgoing> {
        let mut g = self.lock_inner();
        let mut ctx = g.ctx(now)?;
        cert::revoke_cert(&cert_id, &mut ctx)
    }

    // ---------------------------------------------------------------- channels

    /// One lap of a frame as QR matrices (fountain frames of the page's size, with parity).
    pub fn qr_frames(&self, frame: Vec<u8>, parity_percent: u32) -> Result<Vec<QrFrame>> {
        channels::qr_frames(&frame, parity_percent)
    }

    /// A camera frame (luma plane): decodes QR codes and feeds the fountain decoder.
    pub fn scan_gray(&self, w: u32, h: u32, luma: Vec<u8>) -> Vec<ScanEvent> {
        let mut g = self.lock_inner();
        let events = g.scanner.push_gray(w, h, &luma);
        let mut out = Vec::with_capacity(events.len());
        for e in events {
            match e {
                // like the page's `accept`: a document that is not an SSHub frame is ignored and the collector starts over
                ScanEvent::Document { bytes } if !is_sshub_frame(&bytes) => {
                    g.scanner.forget_document();
                    out.push(ScanEvent::Foreign {
                        len: bytes.len() as u32,
                        bytes: if bytes.len() <= 256 {
                            bytes
                        } else {
                            Vec::new()
                        },
                    });
                }
                other => out.push(other),
            }
        }
        out
    }

    /// Forgets a half-read document (the person left the scan screen).
    pub fn scan_reset(&self) {
        self.lock_inner().scanner.reset();
    }

    pub fn audio_burst(&self, frame: Vec<u8>, profile: AudioProfile) -> Result<Vec<i16>> {
        channels::audio_burst(&frame, profile)
    }

    pub fn ble_new(&self, mtu: u32) -> Arc<BleSession> {
        BleSession::new(mtu)
    }

    /// Base64 text of pasted/shared frames (clipboard): the frame, or None when it is not valid base64.
    pub fn frame_from_text(&self, text: String) -> Option<Vec<u8>> {
        from_base64(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};

    fn core() -> Arc<PhoneCore> {
        PhoneCore::with_rng(None, Box::new(ChaCha8Rng::seed_from_u64(5)))
    }

    fn fixture() -> serde_json::Value {
        let p = format!(
            "{}/../../assets/companion/test/fixtures/challenges.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn frame_of(f: &serde_json::Value, name: &str) -> Vec<u8> {
        let c = f["challenges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        hex::decode(c["frame_hex"].as_str().unwrap()).unwrap()
    }

    fn release(typed: Option<&str>) -> Decision {
        Decision::Release {
            options: ReleaseOptions {
                typed_id: typed.map(str::to_string),
                qr: false,
                clipboard: false,
                sound: false,
                audio_profile: AudioProfile::Fast,
                issue_short_code: false,
                by_hand: false,
            },
        }
    }

    #[test]
    fn a_decision_refused_for_the_typed_id_does_not_consume_the_token() {
        let f = fixture();
        let now = f["now"].as_i64().unwrap();
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        let Opened::ShareRequest { view } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!("not a share request");
        };
        let want = view
            .type_to_approve
            .clone()
            .expect("an unmet requester needs its id typed");
        for typed in [None, Some("0000000000000000"), Some("")] {
            let e = c.decide(view.token, release(typed), now + 1).unwrap_err();
            assert!(matches!(e, CoreError::BadInput { .. }), "{typed:?}");
        }
        // the same request is still open and the right id completes it
        let out = c.decide(view.token, release(Some(&want)), now + 1).unwrap();
        assert_eq!(out.kind, 2);
        // a completed decision consumed the token
        assert!(c.decide(view.token, release(Some(&want)), now + 1).is_err());
    }

    #[test]
    fn a_restored_phone_refuses_a_share_request_up_front_and_keeps_the_request_for_after_the_chain_is_taken(
    ) {
        let f = fixture();
        let now = f["now"].as_i64().unwrap();
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        c.lock_inner().unlocked.as_mut().unwrap().state = PhoneState::restored();
        let Opened::ShareRequest { view } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        assert!(view.head_pending);
        let typed = view.type_to_approve.clone();
        let e = c
            .decide(view.token, release(typed.as_deref()), now + 1)
            .unwrap_err();
        assert!(matches!(e, CoreError::Refused { .. }), "{e}");
        assert_eq!(e.reason(), crate::state::HEAD_PENDING_MESSAGE);
        assert_eq!(c.status(now).unwrap().head_seq, None, "nothing was signed");
        // the same request is still open: take the chain from two agreeing lists, accept it, then approve
        let id = PhoneIdentity::new(&[0x42; 32], 0).unwrap();
        let mut live = PhoneState::default();
        chain::head_attest(&mut live, &id, now - 10, None).unwrap();
        let list = |sender: u8| {
            PhoneOpsWire {
                sender: Some([sender; 8]),
                terminals: None,
                ops: chain::courier_ops(&live, 8),
            }
            .encode()
        };
        assert!(!c.take_chain(list(1)).unwrap().agreed);
        let agreed = c.take_chain(list(2)).unwrap();
        assert!(agreed.agreed);
        c.decide(agreed.token, Decision::Approve { typed_id: None }, now)
            .unwrap();
        assert!(!c.status(now).unwrap().head_pending);
        let out = c
            .decide(view.token, release(typed.as_deref()), now + 1)
            .unwrap();
        assert_eq!(out.kind, 2);
    }

    #[test]
    fn a_decision_that_fails_after_the_append_changes_nothing_and_the_next_reply_carries_no_stray_op(
    ) {
        let f = fixture();
        let now = f["now"].as_i64().unwrap();
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        let Opened::ShareRequest { view } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        let typed = view.type_to_approve.clone();
        let sealed_before = c.status(now).unwrap();
        crate::outgoing::fault::FAIL_NEXT_RENDER.with(|f| f.set(true));
        assert!(c
            .decide(view.token, release(typed.as_deref()), now + 1)
            .is_err());
        let after = c.status(now).unwrap();
        assert_eq!(
            after.head_seq, sealed_before.head_seq,
            "no op stayed in the chain"
        );
        assert!(c.export_ops_view().unwrap().is_empty());
        // a later approval carries exactly its own HeadAttest
        let Opened::ShareRequest { view } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        let out = c
            .decide(
                view.token,
                release(view.type_to_approve.as_deref()),
                now + 2,
            )
            .unwrap();
        assert_eq!(out.kind, 2);
        assert_eq!(c.export_ops_view().unwrap().len(), 1);
    }

    /// Measurement, not a check: how long one short-code release holds the core Mutex with the real floor profile.
    /// `cargo test --profile phone -p sshub-phone-core --lib -- --ignored --nocapture mutex_hold_time`
    #[test]
    #[ignore]
    fn mutex_hold_time_of_one_short_code_release() {
        let f = fixture();
        let now = f["now"].as_i64().unwrap();
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        let Opened::ShareRequest { view } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        let typed = view.type_to_approve.clone();
        let mut opts = ReleaseOptions {
            typed_id: typed,
            qr: false,
            clipboard: false,
            sound: false,
            audio_profile: AudioProfile::Fast,
            issue_short_code: true,
            by_hand: false,
        };
        opts.issue_short_code = true;
        let worker = {
            let c = c.clone();
            let token = view.token;
            std::thread::spawn(move || {
                let t = std::time::Instant::now();
                let out = c
                    .decide(token, Decision::Release { options: opts }, now + 1)
                    .unwrap();
                (
                    t.elapsed(),
                    out.short_code
                        .map(|s| (s.argon_mem_mib, s.argon_iters, s.weak)),
                )
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        let t = std::time::Instant::now();
        let _ = c.status(now + 1).unwrap();
        let blocked = t.elapsed();
        let (decide, sc) = worker.join().unwrap();
        println!(
            "MUTEX decide_total_ms={} status_blocked_ms={} short_code(mem_mib,iters,weak)={sc:?}",
            decide.as_millis(),
            blocked.as_millis()
        );
    }

    #[test]
    fn a_garbage_or_refused_frame_leaves_the_pending_requests_intact_and_a_valid_request_replaces_them(
    ) {
        let f = fixture();
        let now = f["now"].as_i64().unwrap();
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        let Opened::ShareRequest { view } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        let typed = view.type_to_approve.clone();
        let magic = WIRE_MAGIC;
        let with_magic = |kind: u8, tail: &[u8]| {
            let mut v = magic.to_vec();
            v.extend_from_slice(&[0, 1, kind]);
            v.extend_from_slice(tail);
            v
        };
        // a garbage frame of each request kind, an unknown kind, a frame without the magic, an enrollment frame
        for bad in [
            with_magic(1, &[9, 9, 9]),
            with_magic(3, &[9, 9, 9]),
            with_magic(5, &[9, 9, 9]),
            with_magic(9, &[9, 9, 9]),
            with_magic(10, &[9, 9, 9]),
            with_magic(99, &[1, 2, 3]),
            vec![1, 2, 3, 4, 5, 6, 7, 8],
            with_magic(6, &[9]),
        ] {
            let o = c.open_frame(bad.clone(), now).unwrap();
            assert!(matches!(o, Opened::Refused { .. }), "{bad:?}: {o:?}");
        }
        // a refused challenge (the page's bad cases) does not wipe it either
        let o = c.open_frame(frame_of(&f, "expired"), now);
        assert!(!matches!(o, Ok(Opened::ShareRequest { .. })), "{o:?}");
        // the first request is still there and completes
        let out = c
            .decide(view.token, release(typed.as_deref()), now + 1)
            .unwrap();
        assert_eq!(out.kind, 2);
        // a valid new request replaces the older one
        let Opened::ShareRequest { view: first } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        let Opened::ShareRequest { view: second } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        assert!(c
            .decide(
                first.token,
                release(second.type_to_approve.as_deref()),
                now + 1
            )
            .is_err());
        assert!(c
            .decide(
                second.token,
                release(second.type_to_approve.as_deref()),
                now + 1
            )
            .is_ok());
    }

    #[test]
    fn lock_then_unlock_in_the_same_process_keeps_the_state_and_a_wrong_secret_is_state_lost_never_empty(
    ) {
        use crate::state::RingDevice;
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        c.never_signed(c.status(0).unwrap().device_id).unwrap();
        {
            let mut g = c.lock_inner();
            let g = &mut *g;
            let u = g.unlocked.as_mut().unwrap();
            for t in 0..2 {
                chain::head_attest(&mut u.state, &u.id, 1_900_000_000 + t, None).unwrap();
            }
            u.state.ring.insert(
                "723ed85f0566df0c".into(),
                RingDevice {
                    name: "terminal A".into(),
                    ed25519_pub: "00".repeat(32),
                    enrolled_at: 5,
                },
            );
            u.state.record_snapshot(
                &"0a".repeat(16),
                crate::state::SnapshotRecord {
                    issued_at: 1_900_000_000,
                    expires_at: 1_900_600_000,
                    mem_kib: 64,
                    iters: 1,
                    weak: false,
                },
            );
        }
        let before = c.lock_inner().unlocked.as_ref().unwrap().state.clone();
        // every seal bumps the version counter `rev`; everything else must be equal
        let same = |c: &PhoneCore, want: &PhoneState| {
            let mut got = c.lock_inner().unlocked.as_ref().unwrap().state.clone();
            got.rev = want.rev;
            got == *want
        };
        // a screen off locks the core; the next unlock in the same process opens the same state
        c.lock();
        let info = c.unlock(vec![0x42; 32]).unwrap();
        assert!(!info.state_lost && !info.head_pending);
        assert_eq!(info.head_seq, Some(2));
        assert_eq!(info.ring.len(), 1);
        assert!(same(&c, &before), "ring, chain, short codes and all");
        // many locks in a row, an unlock that is repeated without a lock in between, still the same state
        c.lock();
        c.lock();
        c.unlock(vec![0x42; 32]).unwrap();
        c.unlock(vec![0x42; 32]).unwrap();
        assert!(same(&c, &before));
        // the bytes the shell stores are the same state
        let stored = c.sealed_state().unwrap();
        c.lock();
        let c2 = PhoneCore::with_rng(Some(stored), Box::new(ChaCha8Rng::seed_from_u64(2)));
        c2.unlock(vec![0x42; 32]).unwrap();
        assert!(same(&c2, &before));
        // the wrong secret after a lock is a lost state (restored, head pending), never a silent empty default
        c.lock();
        let info = c.unlock(vec![0x43; 32]).unwrap();
        assert!(info.state_lost && info.head_pending, "{info:?}");
    }

    #[test]
    fn a_restore_started_while_unlocked_keeps_the_old_state_for_the_old_key() {
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        c.never_signed(c.status(0).unwrap().device_id).unwrap();
        {
            let mut g = c.lock_inner();
            let g = &mut *g;
            let u = g.unlocked.as_mut().unwrap();
            chain::head_attest(&mut u.state, &u.id, 1_900_000_000, None).unwrap();
        }
        let other = core();
        other.unlock(vec![0x11; 32]).unwrap();
        let words = other.backup_words().unwrap();
        c.restore_from_words(words).unwrap();
        // the prompt is cancelled: the old key unlocks and finds its state, not an empty one
        let info = c.unlock(vec![0x42; 32]).unwrap();
        assert!(!info.state_lost && !info.head_pending);
        assert_eq!(info.head_seq, Some(1));
    }

    #[test]
    fn missing_stored_state_after_an_unlock_is_refused_not_started_empty() {
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        // a fresh install started empty; if the bytes were ever lost after that, say so
        {
            let mut g = c.lock_inner();
            g.unlocked = None;
            g.sealed = None;
        }
        let e = c.unlock(vec![0x42; 32]).unwrap_err();
        assert!(matches!(e, CoreError::Refused { .. }), "{e}");
        // a core that never held a state still starts empty
        assert!(core().unlock(vec![0x42; 32]).is_ok());
    }

    #[test]
    fn a_cancelled_restore_does_not_arm_the_next_unlock_of_the_old_key() {
        // the old phone: key B with a real chain, sealed
        let old = core();
        old.unlock(vec![0x42; 32]).unwrap();
        old.never_signed(old.status(0).unwrap().device_id).unwrap();
        {
            let mut g = old.lock_inner();
            let g = &mut *g;
            let u = g.unlocked.as_mut().unwrap();
            for t in 0..2 {
                chain::head_attest(&mut u.state, &u.id, 1_900_000_000 + t, None).unwrap();
            }
        }
        let sealed = old.sealed_state().unwrap();
        // the shell's PhoneCore over that stored state; the words of another key A are typed in
        let c = PhoneCore::with_rng(Some(sealed), Box::new(ChaCha8Rng::seed_from_u64(9)));
        let other = core();
        other.unlock(vec![0x11; 32]).unwrap();
        let words = other.backup_words().unwrap();
        assert_eq!(c.restore_from_words(words.clone()).unwrap(), vec![0x11; 32]);
        // the person cancels the system prompt: the old key stays and unlocks normally, its sealed state opens
        let info = c.unlock(vec![0x42; 32]).unwrap();
        assert!(!info.head_pending && !info.state_lost);
        assert_eq!(info.head_seq, Some(2));
        // the flag is gone: a later unlock of the old key (sealed state given again) is still normal
        c.lock();
        // and a restore that is carried out (the restored secret itself) starts head pending
        assert_eq!(c.restore_from_words(words).unwrap(), vec![0x11; 32]);
        let info = c.unlock(vec![0x11; 32]).unwrap();
        assert!(info.head_pending);
        assert_eq!(info.head_seq, None);
    }

    #[test]
    fn one_wrong_or_stray_frame_during_enrollment_does_not_end_it_but_a_stopping_refusal_does() {
        use sshub_sync_proto::crypto::DeviceIdentity;
        use sshub_sync_proto::enroll_core::Party;
        use sshub_sync_proto::wire::EnrollFrame;
        let now = 1_900_000_000;
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        let t = DeviceIdentity::generate();
        let party = Party::new(&t, "lab terminal", true);
        let commit = EnrollFrame::Commit {
            role: 1,
            commit: party.commit(1),
        }
        .encode();
        let reveal = |bytes: Vec<u8>| {
            EnrollFrame::Reveal {
                role: 1,
                reveal: bytes,
            }
            .encode()
        };
        let kind = |o: &Opened| match o {
            Opened::Refused { view } => view.kind,
            other => panic!("expected a refusal, got {other:?}"),
        };
        let Opened::Enroll { view } = c.open_frame(commit.clone(), now).unwrap() else {
            panic!()
        };
        // a reveal before the phone named itself: wrong order, the enrollment goes on
        let o = c.open_frame(reveal(party.reveal.bytes()), now).unwrap();
        assert_eq!(kind(&o), RefusalKind::WrongOrder);
        // a stray document of an enrollment kind: stray, the enrollment goes on
        let o = c
            .open_frame(vec![0x53, 0x57, 0x52, 0x31, 0, 1, 6, 9], now)
            .unwrap();
        assert_eq!(kind(&o), RefusalKind::StrayFrame);
        // the session is alive: the phone can still be named and commits
        let out = c
            .decide(
                view.token,
                Decision::EnrollName {
                    name: "my phone".into(),
                },
                now,
            )
            .unwrap();
        assert_eq!(out.kind, 6);
        // a wrong frame while waiting for the reveal (a second commitment): wrong order, still alive
        let o = c.open_frame(commit.clone(), now).unwrap();
        assert_eq!(kind(&o), RefusalKind::WrongOrder);
        // a reveal that does not open the commitment stops the enrollment for good
        let mut tampered = party.reveal.clone();
        tampered.nonce[0] ^= 1;
        let o = c.open_frame(reveal(tampered.bytes()), now).unwrap();
        assert_eq!(kind(&o), RefusalKind::Mismatch);
        let e = c
            .decide(view.token, Decision::EnrollSas { matches: true }, now)
            .unwrap_err();
        assert!(
            e.reason().contains("no enrollment is running"),
            "{}",
            e.reason()
        );
    }

    #[test]
    fn the_pending_tokens_are_exactly_what_rust_holds() {
        let f = fixture();
        let now = f["now"].as_i64().unwrap();
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        assert!(c.pending_tokens().is_empty());
        let Opened::ShareRequest { view: a } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(c.pending_tokens(), vec![a.token]);
        // garbage and refused frames change nothing
        let _ = c
            .open_frame(vec![0x53, 0x57, 0x52, 0x31, 0, 1, 1, 9], now)
            .unwrap();
        assert_eq!(c.pending_tokens(), vec![a.token]);
        // a typed-id refusal keeps the token
        let _ = c
            .decide(a.token, release(Some("0000000000000000")), now + 1)
            .unwrap_err();
        assert_eq!(c.pending_tokens(), vec![a.token]);
        // a new request replaces it
        let Opened::ShareRequest { view: b } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(c.pending_tokens(), vec![b.token]);
        assert_ne!(a.token, b.token);
        // a completed decision consumes it
        c.decide(b.token, release(b.type_to_approve.as_deref()), now + 1)
            .unwrap();
        assert!(c.pending_tokens().is_empty());
        let Opened::ShareRequest { .. } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        c.lock();
        assert!(c.pending_tokens().is_empty());
    }

    #[test]
    fn a_failed_wrap_key_rotation_rolls_back_the_state_and_the_identity() {
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        c.never_signed(c.status(0).unwrap().device_id).unwrap();
        let before = c.status(0).unwrap();
        // the rotation appends its op and switches the identity, then rendering the reply fails
        crate::outgoing::fault::FAIL_NEXT_RENDER.with(|f| f.set(true));
        assert!(c.rotate_wrap_keys(1_900_000_000).is_err());
        let after = c.status(0).unwrap();
        assert_eq!(
            (after.wrap_epoch, after.head_seq),
            (before.wrap_epoch, before.head_seq)
        );
        assert_eq!(
            c.lock_inner().unlocked.as_ref().unwrap().id.wrap_epoch(),
            0,
            "the identity is back at generation 0"
        );
        // the same identity still signs for generation 0 and a later rotation works from there
        assert!(c.rotate_wrap_keys(1_900_000_001).is_ok());
        assert_eq!(c.status(0).unwrap().wrap_epoch, 1);
        assert_eq!(c.lock_inner().unlocked.as_ref().unwrap().id.wrap_epoch(), 1);
    }

    #[test]
    fn an_explicit_cancel_and_an_expired_request_consume_the_token() {
        let f = fixture();
        let now = f["now"].as_i64().unwrap();
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        let Opened::ShareRequest { view } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        c.decide(view.token, Decision::Refuse, now).unwrap();
        assert!(
            c.decide(view.token, release(view.type_to_approve.as_deref()), now)
                .is_err(),
            "cancelled"
        );
        let Opened::ShareRequest { view } = c
            .open_frame(frame_of(&f, "unknown-requester"), now)
            .unwrap()
        else {
            panic!()
        };
        let late = view.expires_at;
        let e = c
            .decide(view.token, release(view.type_to_approve.as_deref()), late)
            .unwrap_err();
        assert!(
            matches!(e, CoreError::Refused { .. }),
            "an expired request is refused for good: {e}"
        );
        assert!(c
            .decide(view.token, release(view.type_to_approve.as_deref()), now)
            .is_err());
    }

    #[test]
    fn unlock_info_carries_the_device_name_and_the_epoch() {
        let c = core();
        let i = c.unlock(vec![0x42; 32]).unwrap();
        assert_eq!((i.device_name.clone(), i.ring_epoch), (None, 0));
        {
            let mut g = c.lock_inner();
            let u = g.unlocked.as_mut().unwrap();
            u.state.enrolled = Some(crate::state::Enrolled {
                device_id: "aaaaaaaaaaaaaaaa".into(),
                name: "my\u{202e}phone".into(),
                at: 1,
            });
            u.state.epoch = 7;
        }
        let i = c.status(2).unwrap();
        assert_eq!(
            i.device_name.as_deref(),
            Some("my\\u202ephone"),
            "escaped like every name from a screen"
        );
        assert_eq!(i.ring_epoch, 7);
    }

    fn gray(f: &QrFrame, scale: usize) -> (u32, u32, Vec<u8>) {
        let n = f.size as usize;
        let side = (n + 8) * scale;
        let mut px = vec![255u8; side * side];
        for r in 0..n {
            for c in 0..n {
                if f.modules[r * n + c] == 1 {
                    for y in 0..scale {
                        for x in 0..scale {
                            px[((r + 4) * scale + y) * side + (c + 4) * scale + x] = 0;
                        }
                    }
                }
            }
        }
        (side as u32, side as u32, px)
    }

    fn scan_all(c: &PhoneCore, doc: &[u8]) -> Vec<ScanEvent> {
        let mut seen = Vec::new();
        let frames = channels::qr_frames(doc, 25).unwrap();
        for f in frames.iter().chain(frames.iter()) {
            let (w, h, px) = gray(f, 6);
            seen.extend(c.scan_gray(w, h, px));
        }
        seen
    }

    #[test]
    fn a_qr_that_is_not_an_sshub_frame_is_reported_as_foreign_and_a_real_frame_is_a_document() {
        let c = core();
        let alien: Vec<u8> = (0..300u32).map(|i| (i * 7) as u8).collect();
        let ev = scan_all(&c, &alien);
        assert!(
            ev.iter()
                .any(|e| matches!(e, ScanEvent::Foreign { len: 300, bytes } if bytes.is_empty())),
            "{ev:?}"
        );
        assert!(!ev.iter().any(|e| matches!(e, ScanEvent::Document { .. })));
        // the collector started over: a real frame (magic, version, kind 1) is handed on as a document
        let mut real = vec![0x53, 0x57, 0x52, 0x31, 0, 1, 1];
        real.extend((0..200u32).map(|i| (i * 3) as u8));
        let ev = scan_all(&c, &real);
        assert!(
            ev.iter()
                .any(|e| matches!(e, ScanEvent::Document { bytes } if *bytes == real)),
            "{ev:?}"
        );
        assert!(is_sshub_frame(&real) && !is_sshub_frame(&alien));
        let mut bad_kind = real.clone();
        bad_kind[6] = 13;
        assert!(!is_sshub_frame(&bad_kind));
        bad_kind[6] = 11;
        assert!(
            is_sshub_frame(&bad_kind),
            "kind 11 and 12 are frames since Phase B"
        );
    }

    #[test]
    fn a_small_plain_qr_is_reported_once_as_foreign_with_its_bytes() {
        let c = core();
        let text = b"sshub:dev:1:723ed85f0566df0c".to_vec();
        let qr = channels::qr_matrix(&text).unwrap();
        let (w, h, px) = gray(&qr, 6);
        let ev = c.scan_gray(w, h, px.clone());
        assert!(
            ev.iter()
                .any(|e| matches!(e, ScanEvent::Foreign { len: 28, bytes } if *bytes == text)),
            "{ev:?}"
        );
        // held in front of the camera: reported once, not at every frame
        let again = c.scan_gray(w, h, px.clone());
        assert!(
            !again.iter().any(|e| matches!(e, ScanEvent::Foreign { .. })),
            "{again:?}"
        );
        // after a reset it can be scanned again
        c.scan_reset();
        let third = c.scan_gray(w, h, px);
        assert!(third.iter().any(|e| matches!(e, ScanEvent::Foreign { .. })));
    }

    #[test]
    fn the_chain_view_says_what_the_page_says() {
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        c.lock_inner().unlocked.as_mut().unwrap().state = PhoneState::restored();
        let id = PhoneIdentity::new(&[0x42; 32], 0).unwrap();
        let mut live = PhoneState::default();
        for t in 0..3 {
            chain::head_attest(&mut live, &id, 1_893_452_400 + t, None).unwrap();
        }
        let wire = |n: usize, sender: Option<u8>| {
            PhoneOpsWire {
                sender: sender.map(|s| [s; 8]),
                terminals: None,
                ops: chain::courier_ops(&live, 8).into_iter().take(n).collect(),
            }
            .encode()
        };
        let first = c.take_chain(wire(2, Some(1))).unwrap();
        assert!(!first.agreed);
        assert_eq!(first.sender.as_deref(), Some("0101010101010101"));
        assert_eq!(
            first.summary,
            "Newest op in this list: seq 2, signed 2029-12-31T23:00Z (2 op(s) read)."
        );
        assert_eq!(
            first.note,
            "Take a second list from another terminal before this chain head is trusted."
        );
        let second = c.take_chain(wire(3, Some(2))).unwrap();
        assert!(!second.agreed);
        assert!(
            second
                .note
                .starts_with("The earlier list ended at seq 2, so one of the two is behind."),
            "{}",
            second.note
        );
        // the same frame again (one animation scanned twice) is refused with a sentence, not agreed
        let e = c.take_chain(wire(3, Some(2))).unwrap_err();
        assert_eq!(e.reason(), chain::SAME_TERMINAL_MESSAGE);
        // a list that names no terminal never agrees, and does not cost the candidate its id
        let anon = c.take_chain(wire(3, None)).unwrap();
        assert!(!anon.agreed);
        assert!(
            anon.note
                .starts_with("This list does not say which terminal made it"),
            "{}",
            anon.note
        );
        assert_eq!(
            c.take_chain(wire(3, Some(2))).unwrap_err().reason(),
            chain::SAME_TERMINAL_MESSAGE
        );
        // another terminal naming the same head agrees
        let agreed = c.take_chain(wire(3, Some(3))).unwrap();
        assert!(agreed.agreed);
        assert_eq!(
            agreed.summary,
            "Newest op in this list: seq 3, signed 2029-12-31T23:00Z (3 op(s) read)."
        );
        assert_eq!(agreed.note, "This phone will continue after seq 3. If any terminal holds a higher seq of this phone, do not adopt this: signing on top of an old head forks the chain. Take a newer list instead.");
    }

    #[test]
    fn a_renewal_request_opens_and_is_signed_through_open_frame_and_decide() {
        use sshub_sync_proto::oplog::CertScope;
        use sshub_sync_proto::wire::{CertRenewRequestWire, HeadEvidence};
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        let t0 = 1_893_452_400;
        let cert_id = {
            let mut g = c.lock_inner();
            let u = g.unlocked.as_mut().unwrap();
            u.state = PhoneState::default();
            chain::cert_issue(
                &mut u.state,
                &u.id,
                "0102030405060708".into(),
                CertScope::Hosts(vec![[1; 16]]),
                t0 + 86_400,
                2,
                0,
                1,
                t0,
                None,
            )
            .unwrap()
            .hash
        };
        let now = t0 + 3_600;
        let req = CertRenewRequestWire {
            cert_id,
            expiry: t0 + 2 * 86_400,
            evidence: vec![HeadEvidence {
                terminal: [9; 8],
                head_seq: 3,
                head_ts: now - 60,
            }],
        };
        let Opened::CertRenew { view } = c.open_frame(req.encode(), now).unwrap() else {
            panic!("not a renewal card")
        };
        assert_eq!(view.renew.refusal, None);
        assert_eq!(view.renew.evidence_age_secs, Some(60));
        let out = c
            .decide(view.token, Decision::Approve { typed_id: None }, now)
            .unwrap();
        let ops = PhoneOpsWire::decode(&out.frame).unwrap().ops;
        let last = sshub_sync_proto::oplog::canonical::decode_op(
            &ops.last().unwrap().0,
            ops.last().unwrap().1,
        )
        .unwrap();
        assert!(
            matches!(last.body, sshub_sync_proto::oplog::OpBody::CertRenew { expiry, .. } if expiry == t0 + 2 * 86_400)
        );
    }

    #[test]
    fn one_list_head_needs_the_tick_through_decide() {
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        c.lock_inner().unlocked.as_mut().unwrap().state = PhoneState::restored();
        let id = PhoneIdentity::new(&[0x42; 32], 0).unwrap();
        let mut live = PhoneState::default();
        for t in 0..3 {
            chain::head_attest(&mut live, &id, 1_893_452_400 + t, None).unwrap();
        }
        let list = PhoneOpsWire {
            sender: Some([1; 8]),
            terminals: Some(TerminalsBlock {
                count: 2,
                newest_op_ts: 1_893_452_402,
            }),
            ops: chain::courier_ops(&live, 8),
        }
        .encode();
        let v = c.take_chain(list).unwrap();
        assert!(v.agreed && v.single_source);
        assert_eq!(
            (v.terminals_count, v.terminals_newest_at),
            (Some(2), Some(1_893_452_402))
        );
        assert!(
            v.note
                .starts_with("Your ring has 2 terminals and I can reach only this one."),
            "{}",
            v.note
        );
        let now = 1_893_452_500;
        let e = c
            .decide(v.token, Decision::Approve { typed_id: None }, now)
            .unwrap_err();
        assert_eq!(e.reason(), chain::SINGLE_SOURCE_TICK_MESSAGE);
        // the refusal keeps the card open: the tick is a second tap, not a new scan
        c.decide(v.token, Decision::AcceptSingleSource, now)
            .unwrap();
        assert!(!c.lock_inner().unlocked.as_ref().unwrap().state.head_pending);
    }

    #[test]
    fn the_exported_ops_come_with_their_rows() {
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        c.never_signed(c.status(0).unwrap().device_id).unwrap();
        {
            let mut g = c.lock_inner();
            let g = &mut *g;
            let u = g.unlocked.as_mut().unwrap();
            for t in 0..3 {
                chain::head_attest(&mut u.state, &u.id, 1_900_000_000 + t, None).unwrap();
            }
        }
        let rows = c.export_ops_view().unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows
            .iter()
            .all(|v| v.kind == "HeadAttest" && v.rows.len() >= 3));
        let text = c.export_ops_text().unwrap();
        assert_eq!(
            PhoneOpsWire::decode(&from_base64(&text).unwrap())
                .unwrap()
                .ops
                .len(),
            rows.len()
        );
    }

    #[test]
    fn the_enrollment_name_problem_is_known_before_the_scan() {
        assert_eq!(check_enroll_name("pixel".into()), None);
        assert_eq!(check_enroll_name("  pixel  ".into()), None);
        assert_eq!(
            check_enroll_name("   ".into()).as_deref(),
            Some("Give this phone a name.")
        );
        assert_eq!(
            check_enroll_name("x".repeat(65)).as_deref(),
            Some("The name is longer than 64 bytes.")
        );
        assert_eq!(
            check_enroll_name("a\tb".into()).as_deref(),
            Some("The name has control characters.")
        );
    }

    #[test]
    fn sas_words_pass_is_word() {
        let w = sshub_sync_proto::crypto::sas_words(&[0x5a; 32]);
        for word in w.split('-') {
            assert!(is_word(word.into()), "{word}");
        }
        assert!(!is_word("harbour2".into()) && !is_word("".into()));
    }

    #[test]
    fn a_refused_enrollment_name_leaves_the_enrollment_where_it_was() {
        use sshub_sync_proto::wire::EnrollFrame;
        let c = core();
        c.unlock(vec![0x42; 32]).unwrap();
        let commit = EnrollFrame::Commit {
            role: 1,
            commit: [7; 32],
        }
        .encode();
        let Opened::Enroll { view } = c.open_frame(commit, 10).unwrap() else {
            panic!("not an enrollment step");
        };
        for bad in ["", "   ", "tab\there", "line\nbreak"] {
            let e = c
                .decide(view.token, Decision::EnrollName { name: bad.into() }, 11)
                .unwrap_err();
            assert!(matches!(e, CoreError::BadInput { .. }), "{bad:?}");
        }
        let long = "x".repeat(65);
        assert!(matches!(
            c.decide(view.token, Decision::EnrollName { name: long }, 11)
                .unwrap_err(),
            CoreError::BadInput { .. }
        ));
        // the same flow still takes a good name and answers with the phone commitment (kind 6)
        let out = c
            .decide(
                view.token,
                Decision::EnrollName {
                    name: "my phone".into(),
                },
                12,
            )
            .unwrap();
        assert_eq!(out.kind, 6);
        // and a SAS decision before the terminal revealed its keys is refused without ending the flow
        let e = c
            .decide(view.token, Decision::EnrollSas { matches: true }, 13)
            .unwrap_err();
        assert!(matches!(e, CoreError::BadInput { .. }));
        assert!(
            c.decide(
                view.token,
                Decision::EnrollName {
                    name: "again".into()
                },
                14
            )
            .is_err(),
            "the commitment step is over"
        );
    }

    #[test]
    fn words_are_checked_against_the_bip39_list() {
        for w in ["abandon", "ZOO", " zoo ", "legal"] {
            assert!(is_word(w.into()), "{w}");
        }
        for w in ["", "zzzz", "abandons", "ab and", "тест"] {
            assert!(!is_word(w.into()), "{w}");
        }
    }
}
