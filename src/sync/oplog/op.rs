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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CredMode {
    Vault,
    Quorum,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ThirdFactor {
    Paper,
    HardwareKey(DeviceId),
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
    },
    ShareReshare {
        epoch: u64,
        new_holders: Vec<DeviceId>,
        commitments: Vec<Vec<u8>>,
    },
    ShareReshareConfirm {
        reshare_hash: [u8; 32],
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
    },
    PhoneReenrollCancel {
        reenroll_hash: [u8; 32],
        by_phone: bool,
        third_factor: Option<ThirdFactor>,
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
