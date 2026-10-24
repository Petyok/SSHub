//! The sync round: messages, caps and the two sides of one exchange.
//!
//! One round between two enrolled devices, server speaking first:
//!
//! ```text
//! S Hello          C Hello            versions, nonces, ephemeral keys
//! C Auth           S Auth             signatures over the transcript (possession of the device keys)
//! C Prove          S Prove            pair epoch + MAC under the pair key (the ratchet)
//! C Heads          S Heads            chain heads, self-signed heads, fork proofs
//! C WantTail.. End S Tail..           client pulls
//! S WantTail.. End C Tail..           server pulls
//! C Finish         S Finish           MAC over the whole transcript under the round key
//! C Ack                               server commits the ratchet step
//! ```
//!
//! Every frame is absorbed into one transcript in send order, so a modified, dropped, reordered or
//! injected frame makes the `Finish` MACs fail and nothing is committed. A peer that cannot prove
//! the pair epoch ends the round as `Behind` before any op moves.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use anyhow::{anyhow, bail, ensure, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use x25519_dalek::{EphemeralSecret, PublicKey as XPublic};

use super::crypto::{verify, DeviceIdentity};
use super::limits::{MAX_FRAME_BYTES, MAX_OPS_PER_TAIL};
use super::oplog::{
    DeviceId, DeviceStatus, ForkEvidence, HeadVerdict, IngestOutcome, Op, SignedHead,
};
use super::ratchet::{
    epoch_mac, epoch_mac_verify, load_pair, pair_key_from_exchange, ratchet_epoch, round_nonce,
    save_pair, PairState, Transcript,
};
use crate::store::LauncherStore;

pub const PROTO_MAX: u16 = 1;
pub const PROTO_MIN: u16 = 1;

/// Entries in one `Heads` message (devices in a ring are few; this is a ceiling, not a target).
pub const MAX_HEADS: usize = 256;
/// Signed third-party heads gossiped per round.
pub const MAX_GOSSIP_HEADS: usize = 64;
/// Fork proofs forwarded per round.
pub const MAX_FORK_PROOFS: usize = 16;
/// `WantTail` requests one side may make per pull phase.
pub const MAX_WANTS_PER_PHASE: usize = 1024;
/// Ops one round may deliver in total.
pub const MAX_OPS_PER_ROUND: usize = 200_000;
/// A `Tail` frame is cut to fit this many bytes, well under the frame cap.
const TAIL_FRAME_BUDGET: usize = MAX_FRAME_BYTES / 2;
const MAX_REASON_BYTES: usize = 200;

const CLIENT: u8 = 0;
const SERVER: u8 = 1;

pub(crate) mod hex32 {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(&s)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(|_| D::Error::custom("expected 32 bytes"))
    }
}

pub(crate) mod hex64 {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(&s)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(|_| D::Error::custom("expected 64 bytes"))
    }
}

mod opt_hex32 {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &Option<[u8; 32]>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => s.serialize_str(&hex::encode(v)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 32]>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|s| {
                hex::decode(&s)
                    .map_err(D::Error::custom)?
                    .try_into()
                    .map_err(|_| D::Error::custom("expected 32 bytes"))
            })
            .transpose()
    }
}

pub(crate) mod b64 {
    use super::{Engine, B64};
    use serde::{de::Error, Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&B64.encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        B64.decode(String::deserialize(d)?)
            .map_err(D::Error::custom)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct HeadRef {
    pub device_id: DeviceId,
    pub seq: u64,
    #[serde(with = "hex32")]
    pub hash: [u8; 32],
}

/// A self-signed head on the wire (`sshub/sync/head/v1`).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct WireHead {
    pub device_id: DeviceId,
    pub seq: u64,
    #[serde(with = "hex32")]
    pub hash: [u8; 32],
    #[serde(with = "hex64")]
    pub sig: [u8; 64],
}

impl From<&SignedHead> for WireHead {
    fn from(h: &SignedHead) -> Self {
        Self {
            device_id: h.device_id.clone(),
            seq: h.seq,
            hash: h.hash,
            sig: h.sig,
        }
    }
}

impl From<WireHead> for SignedHead {
    fn from(h: WireHead) -> Self {
        SignedHead {
            device_id: h.device_id,
            seq: h.seq,
            hash: h.hash,
            sig: h.sig,
        }
    }
}

/// An op as it travels: its canonical bytes (the exact signed text) and the signature.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct WireOp {
    #[serde(with = "b64")]
    pub canonical: Vec<u8>,
    #[serde(with = "hex64")]
    pub sig: [u8; 64],
}

impl WireOp {
    pub fn of(op: &Op) -> Self {
        Self {
            canonical: op.canonical_bytes(),
            sig: op.signature,
        }
    }

    /// Strict decode: the canonical form must re-encode byte for byte.
    pub fn decode(&self) -> Result<Op> {
        Op::from_canonical(&self.canonical, self.sig)
    }

    fn cost(&self) -> usize {
        self.canonical.len() * 4 / 3 + 200
    }
}

/// Evidence that a device equivocated, in a form the receiver verifies by itself.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireFork {
    Ops { a: WireOp, b: WireOp },
    OpAndHead { op: WireOp, head: WireHead },
    Heads { a: WireHead, b: WireHead },
}

impl WireFork {
    fn of(e: &ForkEvidence) -> Self {
        match e {
            ForkEvidence::Ops(a, b) => Self::Ops {
                a: WireOp::of(a),
                b: WireOp::of(b),
            },
            ForkEvidence::OpAndHead(op, h) => Self::OpAndHead {
                op: WireOp::of(op),
                head: h.into(),
            },
            ForkEvidence::Heads(a, b) => Self::Heads {
                a: a.into(),
                b: b.into(),
            },
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Hello {
    /// Highest protocol version spoken, and the lowest still accepted.
    pub proto: u16,
    pub min_proto: u16,
    pub device_id: DeviceId,
    #[serde(with = "hex32")]
    pub nonce: [u8; 32],
    /// Ephemeral X25519 public key of this round.
    #[serde(with = "hex32")]
    pub eph: [u8; 32],
    /// Hash of the newest checkpoint this device holds, if any.
    #[serde(default, with = "opt_hex32")]
    pub checkpoint: Option<[u8; 32]>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
pub struct HeadsMsg {
    pub heads: Vec<HeadRef>,
    /// The sender's own freshly signed head.
    #[serde(default)]
    pub own: Option<WireHead>,
    /// Self-signed heads of third devices the sender stored.
    #[serde(default)]
    pub gossip: Vec<WireHead>,
    #[serde(default)]
    pub forks: Vec<WireFork>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum SyncMsg {
    Hello(Hello),
    Auth {
        #[serde(with = "hex64")]
        sig: [u8; 64],
    },
    /// `epoch` is the sender's pair epoch with the receiver (`None`: no history yet).
    Prove {
        epoch: Option<u64>,
        #[serde(with = "hex32")]
        mac: [u8; 32],
    },
    Heads(HeadsMsg),
    WantTail {
        device_id: DeviceId,
        after_seq: u64,
    },
    Tail {
        device_id: DeviceId,
        ops: Vec<WireOp>,
    },
    /// The requested range lies below the sender's prune floor: fetch the checkpoint instead.
    Pruned {
        device_id: DeviceId,
        floor_seq: u64,
    },
    WantCheckpoint {
        #[serde(with = "hex32")]
        hash: [u8; 32],
        offset: u64,
    },
    CheckpointChunk {
        #[serde(with = "hex32")]
        hash: [u8; 32],
        offset: u64,
        total: u64,
        #[serde(with = "b64")]
        data: Vec<u8>,
    },
    EndWants,
    Finish {
        #[serde(with = "hex32")]
        mac: [u8; 32],
    },
    Ack {
        #[serde(with = "hex32")]
        mac: [u8; 32],
    },
    EnrollCommit {
        #[serde(with = "hex32")]
        commit: [u8; 32],
    },
    EnrollReveal(super::enroll::Reveal),
    EnrollConfirm(super::enroll::Confirm),
    Bye {
        reason: String,
    },
}

impl SyncMsg {
    fn name(&self) -> &'static str {
        match self {
            Self::Hello(_) => "hello",
            Self::Auth { .. } => "auth",
            Self::Prove { .. } => "prove",
            Self::Heads(_) => "heads",
            Self::WantTail { .. } => "want_tail",
            Self::Tail { .. } => "tail",
            Self::Pruned { .. } => "pruned",
            Self::WantCheckpoint { .. } => "want_checkpoint",
            Self::CheckpointChunk { .. } => "checkpoint_chunk",
            Self::EndWants => "end_wants",
            Self::Finish { .. } => "finish",
            Self::Ack { .. } => "ack",
            Self::EnrollCommit { .. } => "enroll_commit",
            Self::EnrollReveal(_) => "enroll_reveal",
            Self::EnrollConfirm(_) => "enroll_confirm",
            Self::Bye { .. } => "bye",
        }
    }
}

/// What a finished round did. Hard failures are `Err`; these are the soft outcomes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoundSummary {
    pub peer: DeviceId,
    /// Ops that arrived but wait for approval or a missing dependency.
    pub pending_arrived: usize,
    pub applied: usize,
    pub rejected: usize,
    pub duplicates: usize,
    /// The first few reasons ops were refused, for logs.
    pub reject_reasons: Vec<String>,
    pub received: usize,
    pub sent: usize,
    /// Devices proven to have equivocated this round.
    pub forks: Vec<DeviceId>,
    /// Peers that could not prove the pair epoch; the round moved nothing.
    pub behind: Vec<DeviceId>,
    /// Devices whose heads disagree without proof; ask them directly next time.
    pub suspect: Vec<DeviceId>,
    pub epoch_advanced: bool,
    pub first_contact: bool,
    /// Checkpoint the peer announced.
    pub peer_checkpoint: Option<[u8; 32]>,
    /// Devices whose tail the peer has pruned: bootstrap from its checkpoint.
    pub needs_checkpoint: Vec<DeviceId>,
}

/// A frame pipe. The SSH `Conn` implements it; tests use an in-memory pair.
pub trait SyncLink {
    fn send(&mut self, v: &Value) -> Result<()>;
    fn recv(&mut self) -> Result<Value>;
    /// First frame of a client (carries the connect-to-hello deadline).
    fn recv_first(&mut self) -> Result<Value> {
        self.recv()
    }
}

/// Why a round ended early without being a failure of this side.
#[derive(Debug)]
enum Stop {
    Behind,
    PeerBye(String),
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stop::Behind => write!(f, "pair epoch not provable"),
            Stop::PeerBye(r) => write!(f, "peer ended the round: {r}"),
        }
    }
}

impl std::error::Error for Stop {}

pub(crate) struct Session<'a, L: SyncLink> {
    link: &'a mut L,
    role: u8,
    pub(crate) tr: Transcript,
    first: bool,
}

impl<'a, L: SyncLink> Session<'a, L> {
    pub(crate) fn new(link: &'a mut L, client: bool) -> Self {
        Self {
            link,
            role: if client { CLIENT } else { SERVER },
            tr: Transcript::new(),
            first: client,
        }
    }

    pub(crate) fn send(&mut self, m: &SyncMsg) -> Result<()> {
        let v = serde_json::to_value(m)?;
        self.tr.absorb(self.role, &serde_json::to_vec(&v)?);
        self.link.send(&v)
    }

    pub(crate) fn recv(&mut self) -> Result<SyncMsg> {
        let v = if std::mem::take(&mut self.first) {
            self.link.recv_first()?
        } else {
            self.link.recv()?
        };
        self.tr.absorb(1 - self.role, &serde_json::to_vec(&v)?);
        let m: SyncMsg = serde_json::from_value(v).map_err(|e| anyhow!("malformed frame: {e}"))?;
        if let SyncMsg::Bye { reason } = &m {
            return Err(Stop::PeerBye(reason.chars().take(MAX_REASON_BYTES).collect()).into());
        }
        Ok(m)
    }

    fn bye(&mut self, reason: &str) {
        let _ = self.send(&SyncMsg::Bye {
            reason: reason.into(),
        });
    }

    fn unexpected<T>(&self, want: &str, got: &SyncMsg) -> Result<T> {
        bail!("protocol violation: expected {want}, got {}", got.name())
    }

    fn peer_role(&self) -> u8 {
        1 - self.role
    }
}

struct Peer {
    id: DeviceId,
    ed: [u8; 32],
    nonce: [u8; 32],
    checkpoint: Option<[u8; 32]>,
}

struct Shared {
    peer: Peer,
    round_key: [u8; 32],
    /// The state to advance at the end; `None` for a first contact.
    pair: Option<PairState>,
    claimed: Option<u64>,
    exchange_key: [u8; 32],
    client_nonce: [u8; 32],
    server_nonce: [u8; 32],
}

fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    b
}

fn valid_device_id(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn device_row(store: &LauncherStore, id: &DeviceId) -> Result<Option<super::oplog::DeviceRow>> {
    Ok(store.sync_devices()?.into_iter().find(|d| d.id == *id))
}

/// Hello, Auth and Prove. Returns the shared context or ends the round (`Stop::Behind`, errors).
fn handshake<L: SyncLink>(
    s: &mut Session<'_, L>,
    store: &LauncherStore,
    id: &DeviceIdentity,
    expect: Option<&DeviceId>,
    seen: &mut Option<DeviceId>,
) -> Result<Shared> {
    let me = id.public().device_id();
    let eph = EphemeralSecret::random_from_rng(OsRng);
    let my_hello = Hello {
        proto: PROTO_MAX,
        min_proto: PROTO_MIN,
        device_id: me.clone(),
        nonce: random32(),
        eph: XPublic::from(&eph).to_bytes(),
        checkpoint: None,
    };
    let client = s.role == CLIENT;
    let their_hello = if client {
        let SyncMsg::Hello(h) = s.recv()? else {
            bail!("protocol violation: expected hello")
        };
        s.send(&SyncMsg::Hello(my_hello.clone()))?;
        h
    } else {
        s.send(&SyncMsg::Hello(my_hello.clone()))?;
        match s.recv()? {
            SyncMsg::Hello(h) => h,
            other => return s.unexpected("hello", &other),
        }
    };
    if their_hello.proto < PROTO_MIN
        || their_hello.min_proto > PROTO_MAX
        || their_hello.proto < their_hello.min_proto
    {
        s.bye("no common protocol version");
        bail!(
            "no common protocol version (peer speaks {}..{})",
            their_hello.min_proto,
            their_hello.proto
        );
    }
    if !valid_device_id(&their_hello.device_id) {
        s.bye("bad device id");
        bail!("peer sent a malformed device id");
    }
    if let Some(want) = expect {
        ensure!(
            their_hello.device_id == *want,
            "peer is {}, expected {want}",
            their_hello.device_id
        );
    }
    if their_hello.device_id == me {
        s.bye("talking to myself");
        bail!("peer presents this device's own id");
    }
    let row = match device_row(store, &their_hello.device_id)? {
        Some(r) => r,
        None => {
            s.bye("unknown device");
            bail!("device {} is not enrolled", their_hello.device_id);
        }
    };
    if matches!(
        row.status,
        DeviceStatus::Compromised | DeviceStatus::Revoked
    ) {
        s.bye("device refused");
        bail!("device {} is {:?}", row.id, row.status);
    }
    *seen = Some(row.id.clone());
    let peer = Peer {
        id: row.id.clone(),
        ed: row.public.ed25519_pub,
        nonce: their_hello.nonce,
        checkpoint: their_hello.checkpoint,
    };

    // Auth: sign / verify the transcript so far (both hellos).
    let verify_msg = s.tr.labelled("auth", s.peer_role());
    let sign_msg = s.tr.labelled("auth", s.role);
    let my_auth = SyncMsg::Auth {
        sig: id.sign(&sign_msg),
    };
    let their_sig = if client {
        s.send(&my_auth)?;
        match s.recv()? {
            SyncMsg::Auth { sig } => sig,
            other => return s.unexpected("auth", &other),
        }
    } else {
        let sig = match s.recv()? {
            SyncMsg::Auth { sig } => sig,
            other => return s.unexpected("auth", &other),
        };
        s.send(&my_auth)?;
        sig
    };
    // The server's signature covers the client's auth too in its own view; both signed the same
    // snapshot (taken before either auth frame), so verification uses the snapshot.
    if !verify(&peer.ed, &verify_msg, &their_sig) {
        bail!("peer failed to prove possession of its device key");
    }

    // Prove: pair epoch and MAC.
    let (client_nonce, server_nonce) = if client {
        (my_hello.nonce, peer.nonce)
    } else {
        (peer.nonce, my_hello.nonce)
    };
    let shared_secret = eph.diffie_hellman(&XPublic::from(their_hello.eph));
    let hello_hash = {
        let mut t = Transcript::new();
        t.absorb(CLIENT, &client_nonce);
        t.absorb(SERVER, &server_nonce);
        t.hash()
    };
    let exchange_key = pair_key_from_exchange(shared_secret.as_bytes(), &hello_hash);
    let state = load_pair(store, &peer.id)?;
    let my_claim = state.as_ref().map(|p| p.epoch);
    let my_key = state.as_ref().map_or(exchange_key, |p| p.key);
    let prove_sign = s.tr.labelled("prove", s.role);
    let (claimed, their_mac) = if client {
        s.send(&SyncMsg::Prove {
            epoch: my_claim,
            mac: epoch_mac(&my_key, &prove_sign),
        })?;
        let snapshot = s.tr.labelled("prove", s.peer_role());
        match s.recv()? {
            SyncMsg::Prove { epoch, mac } => (epoch, (mac, snapshot)),
            other => return s.unexpected("prove", &other),
        }
    } else {
        let snapshot = s.tr.labelled("prove", s.peer_role());
        let got = match s.recv()? {
            SyncMsg::Prove { epoch, mac } => (epoch, (mac, snapshot)),
            other => return s.unexpected("prove", &other),
        };
        let sign = s.tr.labelled("prove", s.role);
        s.send(&SyncMsg::Prove {
            epoch: my_claim,
            mac: epoch_mac(&my_key, &sign),
        })?;
        got
    };

    // Verify the peer's claim against my state.
    let verifying_key = match (&state, claimed) {
        (None, None) => Some(exchange_key),
        (Some(p), Some(c)) => p.key_for_claim(c, client),
        _ => None,
    };
    let proven = verifying_key.is_some_and(|k| epoch_mac_verify(&k, &their_mac.1, &their_mac.0));
    if !proven {
        s.bye("behind");
        return Err(Stop::Behind.into());
    }
    // The round key is the client's: its claim fixes the epoch both sides use.
    let client_claim = if client { my_claim } else { claimed };
    let round_key = match (&state, client_claim) {
        (Some(p), Some(c)) if !client => p.key_for_claim(c, false).expect("verified above"),
        (Some(p), _) => p.key,
        (None, _) => exchange_key,
    };
    let mut pair = state;
    if let (false, Some(p), Some(c)) = (client, pair.as_mut(), client_claim) {
        p.settle(c);
    }
    Ok(Shared {
        peer,
        round_key,
        pair,
        claimed,
        exchange_key,
        client_nonce,
        server_nonce,
    })
}

fn heads_msg(store: &LauncherStore, id: &DeviceIdentity) -> Result<HeadsMsg> {
    let me = id.public().device_id();
    let chain = store.sync_chain_heads()?;
    let own = chain
        .iter()
        .find(|(d, _, _)| *d == me)
        .map(|(_, seq, hash)| WireHead::from(&SignedHead::sign(id, *seq, *hash)));
    let heads = chain
        .iter()
        .take(MAX_HEADS)
        .map(|(d, s, h)| HeadRef {
            device_id: d.clone(),
            seq: *s,
            hash: *h,
        })
        .collect();
    let gossip = store.with_conn(|conn| {
        let mut st = conn.prepare(
            "SELECT device_id, MAX(seq), chain_hash, signature FROM sync_head_attests
             WHERE length(signature) = 64 AND device_id != ?1 GROUP BY device_id LIMIT ?2",
        )?;
        let rows = st
            .query_map(rusqlite::params![me, MAX_GOSSIP_HEADS as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(d, s, h, sig)| {
                Some(WireHead {
                    device_id: d,
                    seq: s as u64,
                    hash: h.try_into().ok()?,
                    sig: sig.try_into().ok()?,
                })
            })
            .collect::<Vec<_>>())
    })?;
    let mut forks = Vec::new();
    for d in store.sync_devices()? {
        if d.status == DeviceStatus::Compromised && forks.len() < MAX_FORK_PROOFS {
            if let Some(e) = store.sync_fork_evidence(&d.id)? {
                forks.push(WireFork::of(&e));
            }
        }
    }
    Ok(HeadsMsg {
        heads,
        own,
        gossip,
        forks,
    })
}

fn note_fork(store: &LauncherStore, dev: &DeviceId, sum: &mut RoundSummary) -> Result<()> {
    if device_row(store, dev)?.is_some_and(|d| d.status == DeviceStatus::Compromised)
        && !sum.forks.contains(dev)
    {
        sum.forks.push(dev.clone());
    }
    Ok(())
}

fn check_len(what: &str, n: usize, max: usize) -> Result<()> {
    ensure!(
        n <= max,
        "protocol violation: {what} has {n} entries, the cap is {max}"
    );
    Ok(())
}

/// Gossip and fork proofs from a `Heads` message; unsigned head claims that contradict ours.
fn absorb_heads(store: &LauncherStore, h: &HeadsMsg, sum: &mut RoundSummary) -> Result<()> {
    check_len("heads", h.heads.len(), MAX_HEADS)?;
    check_len("gossip", h.gossip.len(), MAX_GOSSIP_HEADS)?;
    check_len("fork proofs", h.forks.len(), MAX_FORK_PROOFS)?;
    for head in h.own.iter().chain(h.gossip.iter()) {
        if !valid_device_id(&head.device_id) {
            bail!("protocol violation: malformed device id in a signed head");
        }
        let sh: SignedHead = head.clone().into();
        if matches!(store.sync_record_head(&sh)?, HeadVerdict::Fork) {
            note_fork(store, &sh.device_id, sum)?;
        }
    }
    Ok(())
}

/// Fork proofs a peer forwarded. Run after the pulls, so the chains they refer to are here.
fn absorb_forks(store: &LauncherStore, h: &HeadsMsg, sum: &mut RoundSummary) -> Result<()> {
    for f in &h.forks {
        match f {
            WireFork::Ops { a, b } => {
                let (a, b) = (a.decode()?, b.decode()?);
                let dev = a.device_id.clone();
                store.sync_ingest(a)?;
                store.sync_ingest(b)?;
                note_fork(store, &dev, sum)?;
            }
            WireFork::OpAndHead { op, head } => {
                let op = op.decode()?;
                let dev = op.device_id.clone();
                store.sync_ingest(op)?;
                store.sync_record_head(&head.clone().into())?;
                note_fork(store, &dev, sum)?;
            }
            WireFork::Heads { a, b } => {
                store.sync_record_head(&a.clone().into())?;
                store.sync_record_head(&b.clone().into())?;
                note_fork(store, &a.device_id, sum)?;
            }
        }
    }
    Ok(())
}

fn my_heads(store: &LauncherStore) -> Result<BTreeMap<DeviceId, (u64, [u8; 32])>> {
    Ok(store
        .sync_chain_heads()?
        .into_iter()
        .map(|(d, s, h)| (d, (s, h)))
        .collect())
}

/// Sort per chain, ingest, and retry what failed only for order (unknown author, gap) once the rest landed.
fn ingest_ops(store: &LauncherStore, ops: Vec<Op>, sum: &mut RoundSummary) -> Result<bool> {
    let mut queue = ops;
    queue.sort_by(|a, b| (&a.device_id, a.seq).cmp(&(&b.device_id, b.seq)));
    let mut any = false;
    for _pass in 0..4 {
        let mut retry = Vec::new();
        let mut progress = false;
        for op in queue {
            match store.sync_ingest(op.clone())? {
                IngestOutcome::Applied => {
                    sum.applied += 1;
                    progress = true;
                }
                IngestOutcome::Pending => {
                    sum.pending_arrived += 1;
                    progress = true;
                }
                IngestOutcome::ForkProof(a, _) => {
                    note_fork(store, &a.device_id, sum)?;
                    progress = true;
                }
                IngestOutcome::Rejected(why) if why == "duplicate op" => sum.duplicates += 1,
                IngestOutcome::Rejected(why)
                    if why == "unknown device" || why.starts_with("gap:") =>
                {
                    retry.push(op)
                }
                IngestOutcome::Rejected(why) => {
                    sum.rejected += 1;
                    if sum.reject_reasons.len() < 8 {
                        sum.reject_reasons.push(why);
                    }
                }
            }
        }
        any |= progress;
        if retry.is_empty() || !progress {
            sum.rejected += retry.len();
            break;
        }
        queue = retry;
    }
    Ok(any)
}

/// Ask for every tail the peer has beyond ours; ingest as it arrives.
fn pull<L: SyncLink>(
    s: &mut Session<'_, L>,
    store: &LauncherStore,
    theirs: &[HeadRef],
    sum: &mut RoundSummary,
) -> Result<()> {
    let mut asked = 0usize;
    let mut total = 0usize;
    let mut conflicts: BTreeSet<DeviceId> = BTreeSet::new();
    for _pass in 0..64 {
        let mine = my_heads(store)?;
        let mut progressed = false;
        for h in theirs {
            let (have, my_hash) = mine.get(&h.device_id).copied().unwrap_or((0, [0; 32]));
            let after = if h.seq > have {
                have
            } else if h.seq == have
                && have > 0
                && h.hash != my_hash
                && conflicts.insert(h.device_id.clone())
            {
                have - 1
            } else {
                continue;
            };
            asked += 1;
            check_len("want_tail requests", asked, MAX_WANTS_PER_PHASE)?;
            s.send(&SyncMsg::WantTail {
                device_id: h.device_id.clone(),
                after_seq: after,
            })?;
            match s.recv()? {
                SyncMsg::Tail { device_id, ops } => {
                    ensure!(
                        device_id == h.device_id,
                        "protocol violation: tail for the wrong device"
                    );
                    check_len("tail", ops.len(), MAX_OPS_PER_TAIL)?;
                    total += ops.len();
                    check_len("ops this round", total, MAX_OPS_PER_ROUND)?;
                    let decoded = ops
                        .iter()
                        .map(|w| w.decode())
                        .collect::<Result<Vec<_>>>()
                        .map_err(|e| anyhow!("protocol violation: malformed op: {e}"))?;
                    for op in &decoded {
                        ensure!(
                            op.device_id == h.device_id && op.seq > after,
                            "protocol violation: op outside the requested range"
                        );
                    }
                    sum.received += decoded.len();
                    progressed |= ingest_ops(store, decoded, sum)?;
                }
                SyncMsg::Pruned { device_id, .. } => {
                    ensure!(
                        device_id == h.device_id,
                        "protocol violation: pruned for the wrong device"
                    );
                    if !sum.needs_checkpoint.contains(&device_id) {
                        sum.needs_checkpoint.push(device_id);
                    }
                }
                other => return s.unexpected("tail", &other),
            }
        }
        if !progressed {
            break;
        }
    }
    s.send(&SyncMsg::EndWants)?;
    for (dev, (seq, hash)) in my_heads(store)? {
        if let Some(h) = theirs.iter().find(|h| h.device_id == dev) {
            if h.seq == seq
                && h.hash != hash
                && !sum.forks.contains(&dev)
                && !sum.suspect.contains(&dev)
            {
                store.sync_mark_suspect(&dev)?;
                sum.suspect.push(dev);
            }
        }
    }
    Ok(())
}

fn serve_wants<L: SyncLink>(
    s: &mut Session<'_, L>,
    store: &LauncherStore,
    sum: &mut RoundSummary,
) -> Result<()> {
    let mut n = 0usize;
    loop {
        match s.recv()? {
            SyncMsg::WantTail {
                device_id,
                after_seq,
            } => {
                n += 1;
                check_len("want_tail requests", n, MAX_WANTS_PER_PHASE)?;
                ensure!(
                    valid_device_id(&device_id),
                    "protocol violation: malformed device id in want_tail"
                );
                let mut ops = Vec::new();
                let mut bytes = 0usize;
                for op in store.sync_chain_tail(&device_id, after_seq)? {
                    let w = WireOp::of(&op);
                    if !ops.is_empty() && bytes + w.cost() > TAIL_FRAME_BUDGET {
                        break;
                    }
                    bytes += w.cost();
                    ops.push(w);
                    if ops.len() == MAX_OPS_PER_TAIL {
                        break;
                    }
                }
                sum.sent += ops.len();
                s.send(&SyncMsg::Tail { device_id, ops })?;
            }
            SyncMsg::EndWants => return Ok(()),
            other => return s.unexpected("want_tail or end_wants", &other),
        }
    }
}

fn exchange<L: SyncLink>(
    s: &mut Session<'_, L>,
    store: &LauncherStore,
    id: &DeviceIdentity,
    client: bool,
    sum: &mut RoundSummary,
) -> Result<()> {
    let mine = heads_msg(store, id)?;
    let theirs = if client {
        s.send(&SyncMsg::Heads(mine))?;
        match s.recv()? {
            SyncMsg::Heads(h) => h,
            other => return s.unexpected("heads", &other),
        }
    } else {
        let h = match s.recv()? {
            SyncMsg::Heads(h) => h,
            other => return s.unexpected("heads", &other),
        };
        s.send(&SyncMsg::Heads(mine))?;
        h
    };
    absorb_heads(store, &theirs, sum)?;
    if client {
        pull(s, store, &theirs.heads, sum)?;
        serve_wants(s, store, sum)?;
    } else {
        serve_wants(s, store, sum)?;
        // The client's heads may have moved by what we just ingested from forks; use the received list.
        pull(s, store, &theirs.heads, sum)?;
    }
    absorb_forks(store, &theirs, sum)
}

fn finish<L: SyncLink>(
    s: &mut Session<'_, L>,
    store: &LauncherStore,
    ctx: Shared,
    sum: &mut RoundSummary,
) -> Result<()> {
    let client = s.role == CLIENT;
    let next_of =
        |key: &[u8; 32]| ratchet_epoch(key, &round_nonce(&ctx.client_nonce, &ctx.server_nonce));
    let peer = ctx.peer.id.clone();
    if client {
        let mac = epoch_mac(&ctx.round_key, &s.tr.labelled("finish", CLIENT));
        s.send(&SyncMsg::Finish { mac })?;
        let want = s.tr.labelled("finish", SERVER);
        let SyncMsg::Finish { mac } = s.recv()? else {
            bail!("protocol violation: expected finish")
        };
        ensure!(
            epoch_mac_verify(&ctx.round_key, &want, &mac),
            "transcript MAC mismatch: the round was altered"
        );
        let (state, advanced) = match ctx.pair {
            Some(p) => (
                PairState {
                    epoch: p.epoch + 1,
                    key: next_of(&ctx.round_key),
                    next: None,
                    prev: Some(ctx.round_key),
                },
                true,
            ),
            None => (PairState::fresh(ctx.exchange_key), false),
        };
        save_pair(store, &peer, &state)?;
        sum.epoch_advanced = advanced;
        let ack = epoch_mac(&ctx.round_key, &s.tr.labelled("ack", CLIENT));
        s.send(&SyncMsg::Ack { mac: ack })?;
    } else {
        let want = s.tr.labelled("finish", CLIENT);
        let SyncMsg::Finish { mac } = s.recv()? else {
            bail!("protocol violation: expected finish")
        };
        ensure!(
            epoch_mac_verify(&ctx.round_key, &want, &mac),
            "transcript MAC mismatch: the round was altered"
        );
        let mac = epoch_mac(&ctx.round_key, &s.tr.labelled("finish", SERVER));
        s.send(&SyncMsg::Finish { mac })?;
        let ack_label = s.tr.labelled("ack", CLIENT);
        // Provisional until the acknowledgement arrives: a round that dies here must not strand
        // an honest client that already committed.
        let provisional = ctx.pair.map(|mut p| {
            p.next = Some(next_of(&ctx.round_key));
            p
        });
        match &provisional {
            Some(p) => save_pair(store, &peer, p)?,
            // First contact: keep the new key at once, the client holds it already.
            None => save_pair(store, &peer, &PairState::fresh(ctx.exchange_key))?,
        }
        let SyncMsg::Ack { mac } = s.recv()? else {
            bail!("protocol violation: expected ack")
        };
        ensure!(
            epoch_mac_verify(&ctx.round_key, &ack_label, &mac),
            "acknowledgement MAC mismatch"
        );
        let state = match provisional {
            Some(p) => PairState {
                epoch: p.epoch + 1,
                key: p.next.unwrap(),
                next: None,
                prev: None,
            },
            None => PairState::fresh(ctx.exchange_key),
        };
        save_pair(store, &peer, &state)?;
        sum.epoch_advanced = ctx.claimed.is_some();
    }
    Ok(())
}

fn conclude(
    res: Result<()>,
    mut sum: RoundSummary,
    peer_hint: Option<&DeviceId>,
) -> Result<RoundSummary> {
    match res {
        Ok(()) => Ok(sum),
        Err(e) => match e.downcast_ref::<Stop>() {
            Some(Stop::Behind) => {
                if let Some(p) = peer_hint {
                    sum.behind.push(p.clone());
                }
                Ok(sum)
            }
            Some(Stop::PeerBye(r)) if r == "behind" => {
                if let Some(p) = peer_hint {
                    sum.behind.push(p.clone());
                }
                Ok(sum)
            }
            _ => Err(e),
        },
    }
}

fn round<L: SyncLink>(
    link: &mut L,
    store: &LauncherStore,
    id: &DeviceIdentity,
    expect: Option<&DeviceId>,
    client: bool,
    sum: &mut RoundSummary,
    peer_seen: &mut Option<DeviceId>,
) -> Result<()> {
    let mut s = Session::new(link, client);
    let ctx = handshake(&mut s, store, id, expect, peer_seen)?;
    sum.peer = ctx.peer.id.clone();
    sum.first_contact = ctx.pair.is_none();
    sum.peer_checkpoint = ctx.peer.checkpoint;
    exchange(&mut s, store, id, client, sum)?;
    finish(&mut s, store, ctx, sum)
}

/// Client side: one round against `peer` over `conn`.
pub fn run_round<L: SyncLink>(
    conn: &mut L,
    store: &LauncherStore,
    id: &DeviceIdentity,
    peer: &DeviceId,
) -> Result<RoundSummary> {
    let mut sum = RoundSummary {
        peer: peer.clone(),
        ..Default::default()
    };
    let mut seen = Some(peer.clone());
    let res = round(conn, store, id, Some(peer), true, &mut sum, &mut seen);
    conclude(res, sum, Some(peer))
}

/// Server side (`sshub sync-serve`): answer one round from any enrolled device.
pub fn run_sync_serve<L: SyncLink>(
    store: &LauncherStore,
    id: &DeviceIdentity,
    conn: &mut L,
) -> Result<RoundSummary> {
    let mut sum = RoundSummary::default();
    let mut seen = None;
    let res = round(conn, store, id, None, false, &mut sum, &mut seen);
    conclude(res, sum, seen.as_ref())
}

#[cfg(test)]
pub(crate) mod tests;
