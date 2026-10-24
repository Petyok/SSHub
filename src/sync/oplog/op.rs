//! Op types of the replicated log (contract section 2). Wire and signature
//! form: [`super::canonical`].

use crate::store::{GroupUid, HostUid, IdentityUid};
use crate::sync::crypto::{chain_hash, verify, AlgTag, DevicePublic};

use super::canonical;
use super::tier;

/// `hex(ed25519_pub[..8])`.
pub type DeviceId = String;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceRole {
    Holder,
    Mirror,
}

/// Unattended devices never vote at Red.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Attendance {
    Attended,
    Unattended,
}

/// How a device was admitted: replicated so every device shows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerifiedBy {
    Sas,
    SshHostKey,
}

/// Only terminals keep a store and can sign checkpoints; recovery needs to know
/// which device is the phone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceKind {
    Terminal,
    Phone,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CredMode {
    Vault,
    Quorum,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ThirdFactor {
    Paper,
    EmailCode,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WrappedShare {
    pub recipient: DeviceId,
    pub alg: AlgTag,
    pub sealed: Vec<u8>,
}

/// `everything` does not exist: a certificate always names its scope.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CertScope {
    Group(GroupUid),
    Hosts(Vec<HostUid>),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReshareItem {
    pub host_uid: HostUid,
    pub sub_shares: Vec<(DeviceId, Vec<u8>)>,
    pub commitments: Vec<Vec<u8>>,
}

/// Where a checkpoint cuts one device's chain.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CutEntry {
    pub device_id: DeviceId,
    pub seq: u64,
    pub op_hash: [u8; 32],
    pub lamport: u64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConfirmItem {
    pub host_uid: HostUid,
    pub sealed_share: Vec<u8>,
}

/// An unsigned claim "device `device_id` had head `chain_hash` at `seq`".
/// Self-signed attestations (fork proof material) are [`super::fork::SignedHead`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HeadAttest {
    pub device_id: DeviceId,
    pub seq: u64,
    pub chain_hash: [u8; 32],
}

/// Patch of a host: `None` leaves a field untouched, `Some(None)` clears it.
/// Hop tokens in `proxy_jump` that name a saved host travel as `#<uid hex>`.
/// `transport` is the DB string (`ssh`, `mosh`). `group_uids` is sorted, unique
/// and never contains the Favorites uid (that is `favorite`).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct HostFields {
    pub name: Option<String>,
    pub label: Option<Option<String>>,
    pub address: Option<String>,
    pub port: Option<Option<u16>>,
    pub username: Option<Option<String>>,
    pub proxy_jump: Option<Option<String>>,
    pub forward_agent: Option<Option<bool>>,
    pub remote_command: Option<Option<String>>,
    pub transport: Option<Option<String>>,
    pub environment: Option<Option<String>>,
    pub os_icon: Option<Option<String>>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<Option<String>>,
    pub favorite: Option<bool>,
    pub secret_lookup: Option<Option<String>>,
    pub has_password: Option<bool>,
    pub group_uids: Option<Vec<GroupUid>>,
    pub identity_uid: Option<Option<IdentityUid>>,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct GroupFields {
    pub name: Option<String>,
    pub default_identity_uid: Option<Option<IdentityUid>>,
    pub default_username: Option<Option<String>>,
    pub default_port: Option<Option<u16>>,
    pub default_proxy_jump: Option<Option<String>>,
    pub default_transport: Option<Option<String>>,
    pub default_forward_agent: Option<Option<bool>>,
    pub parent_uid: Option<Option<GroupUid>>,
}

/// Identity metadata only; key bytes travel as a credential.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct IdentityFields {
    pub name: Option<String>,
    pub username: Option<Option<String>>,
    pub has_password: Option<bool>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OpBody {
    HostUpsert {
        host_uid: HostUid,
        fields: HostFields,
    },
    HostDelete {
        host_uid: HostUid,
    },
    GroupUpsert {
        group_uid: GroupUid,
        fields: GroupFields,
    },
    GroupDelete {
        group_uid: GroupUid,
    },
    IdentityUpsert {
        identity_uid: IdentityUid,
        fields: IdentityFields,
    },
    IdentityDelete {
        identity_uid: IdentityUid,
    },
    CredUpdate {
        host_uid: HostUid,
        ciphertext: Vec<u8>,
        wrapped_shares: Vec<WrappedShare>,
    },
    DeviceAdd {
        device: DevicePublic,
        name: String,
        role: DeviceRole,
        attendance: Attendance,
        kind: DeviceKind,
        verified_by: VerifiedBy,
    },
    DeviceRevoke {
        device_id: DeviceId,
    },
    /// The requester is in the body as well as the header: a holder that checks
    /// only the body must still see who asks.
    ShareRelease {
        host_uid: HostUid,
        request_id: u64,
        requester: DeviceId,
        /// Ephemeral key the grant is sealed to; never a long-lived key.
        eph_x25519_pub: [u8; 32],
    },
    /// A holder's answer to a `ShareRelease`: `sealed = seal_to(eph, share)`,
    /// at most 128 bytes. Inert: it applies nothing.
    ShareGrant {
        host_uid: HostUid,
        request_id: u64,
        requester: DeviceId,
        epoch: u64,
        sealed: Vec<u8>,
    },
    /// One per (contributor, recipient) per epoch: `hybrid_wrap(to.pub, session key)`.
    /// The contributor is the op's device.
    ShareReshareSession {
        epoch: u64,
        to: DeviceId,
        sealed: Vec<u8>,
    },
    /// One per holder per epoch: `hybrid_wrap(own current pub, storage key)`.
    ShareStorageKey {
        epoch: u64,
        sealed: Vec<u8>,
    },
    /// At most `RESHARE_ITEMS_PER_OP` items.
    ShareReshareBatch {
        epoch: u64,
        new_holders: Vec<DeviceId>,
        items: Vec<ReshareItem>,
    },
    /// The holder's new share per credential; at most `RESHARE_ITEMS_PER_OP` items.
    ShareReshareConfirmBatch {
        epoch: u64,
        items: Vec<ConfirmItem>,
    },
    /// A device announces fresh wrap keys (self op). The signing key never rotates.
    WrapKeyRotate {
        device_id: DeviceId,
        wrap_epoch: u64,
        x25519_pub: [u8; 32],
        mlkem_pub: Vec<u8>,
    },
    CertIssue {
        device_id: DeviceId,
        scope: CertScope,
        expiry: i64,
        max_new_hosts_per_day: u32,
        epoch_lo: u64,
        epoch_hi: u64,
    },
    CertRevoke {
        cert_id: [u8; 32],
    },
    CertHostOpened {
        cert_id: [u8; 32],
        host_uid: HostUid,
    },
    PhoneReenroll {
        initiated_at: i64,
        signers: Vec<DeviceId>,
        via_paper: bool,
        third_factor: ThirdFactor,
        liveness: Vec<HeadAttest>,
        /// Signature by the paper/recover key over the reenroll body without
        /// this field, domain-tagged (checked by stage 3).
        paper_sig: Option<[u8; 64]>,
    },
    PhoneReenrollCancel {
        reenroll_hash: [u8; 32],
        by_phone: bool,
        third_factor: Option<ThirdFactor>,
    },
    /// An attended device approves a pending re-enrollment after the user said so.
    RecoveryApprove {
        reenroll_hash: [u8; 32],
    },
    RecoveryAck {
        reenroll_hash: [u8; 32],
        seen_at: i64,
    },
    ModeChange {
        to: CredMode,
        migrated_on: DeviceId,
        cosigner: Option<DeviceId>,
    },
    HeadAttest {
        device_id: DeviceId,
        seq: u64,
        chain_hash: [u8; 32],
    },
    /// Certifies the resolved state at a cut of the log (C15). Inert until
    /// verified by every device that signs.
    CheckpointPropose {
        cut: Vec<CutEntry>,
        state_hash: [u8; 32],
        state_len: u64,
        prev_checkpoint: [u8; 32],
        required: Vec<DeviceId>,
    },
    CheckpointSign {
        proposal_hash: [u8; 32],
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Op {
    pub device_id: DeviceId,
    pub seq: u64,
    pub lamport: u64,
    pub prev_hash: [u8; 32],
    pub timestamp: i64,
    pub epoch: u64,
    pub alg: AlgTag,
    pub body: OpBody,
    pub signature: [u8; 64],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpTier {
    Green,
    Yellow,
    Red,
}

impl Op {
    /// Everything except the signature, deterministic (contract C2).
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical::encode_op(self)
    }

    pub fn hash(&self) -> [u8; 32] {
        chain_hash(&self.canonical_bytes())
    }

    pub fn verify(&self, signer_pub: &[u8; 32]) -> bool {
        verify(signer_pub, &self.canonical_bytes(), &self.signature)
    }

    pub fn tier(&self) -> OpTier {
        tier::body_tier(&self.body)
    }

    /// Parse canonical bytes plus signature. Strict: bytes that are not
    /// exactly what [`Self::canonical_bytes`] would write are refused, so the
    /// hash and signature always cover the bytes that were received.
    pub fn from_canonical(bytes: &[u8], signature: [u8; 64]) -> anyhow::Result<Self> {
        canonical::decode_op(bytes, signature)
    }
}
