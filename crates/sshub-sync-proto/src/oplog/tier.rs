//! One table classifying every replicated field (contract C5), and the tier of
//! every op body. A field that is not in the table does not replicate.

use super::op::{OpBody, OpTier};
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldClass {
    /// Auto-applied, merged per field, last writer wins.
    Safe,
    /// Held for review as a whole entity change.
    Dangerous,
    /// Never leaves the device.
    LocalOnly,
}

use FieldClass::{Dangerous, LocalOnly, Safe};

/// `ManagedHost` fields (struct names, so a new field fails the exhaustiveness
/// test) plus the wire names that stand in for local row ids.
pub const HOST_FIELDS: &[(&str, FieldClass)] = &[
    ("id", LocalOnly),
    ("name", Safe),
    ("label", Safe),
    ("address", Dangerous),
    ("port", Dangerous),
    ("group_id", LocalOnly),
    ("identity_id", LocalOnly),
    ("group", LocalOnly),
    ("groups", LocalOnly),
    ("identity", LocalOnly),
    ("os_icon", Safe),
    ("tags", Safe),
    ("notes", Safe),
    ("proxy_jump", Dangerous),
    ("forward_agent", Dangerous),
    ("remote_command", Dangerous),
    ("environment", Dangerous),
    ("sort_order", LocalOnly),
    ("favorite", Safe),
    ("last_connected", LocalOnly),
    ("source", LocalOnly),
    ("ssh_config_hash", LocalOnly),
    ("has_password", Dangerous),
    ("secret_lookup", Dangerous),
    ("username", Dangerous),
    ("session_logging", LocalOnly),
    ("ping", LocalOnly),
    ("transport", Dangerous),
    ("created_at", LocalOnly),
    ("updated_at", LocalOnly),
    // wire names
    ("group_uids", Dangerous),
    ("identity_uid", Dangerous),
];

/// `HostGroup` fields plus wire names.
pub const GROUP_FIELDS: &[(&str, FieldClass)] = &[
    ("id", LocalOnly),
    ("name", Safe),
    ("sort_order", LocalOnly),
    ("default_identity_id", LocalOnly),
    ("default_username", Dangerous),
    ("default_port", Dangerous),
    ("default_proxy_jump", Dangerous),
    ("default_transport", Dangerous),
    ("default_forward_agent", Dangerous),
    ("parent_id", LocalOnly),
    ("reserved", LocalOnly),
    // wire names
    ("default_identity_uid", Dangerous),
    ("parent_uid", Dangerous),
];

/// `Identity` fields plus wire names. Key file paths never replicate.
pub const IDENTITY_FIELDS: &[(&str, FieldClass)] = &[
    ("id", LocalOnly),
    ("name", Safe),
    ("username", Dangerous),
    ("private_key", LocalOnly),
    ("certificate", LocalOnly),
    ("has_password", Dangerous),
];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum EntityKind {
    Host,
    Group,
    Identity,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Host => "host",
            EntityKind::Group => "group",
            EntityKind::Identity => "identity",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "host" => Some(Self::Host),
            "group" => Some(Self::Group),
            "identity" => Some(Self::Identity),
            _ => None,
        }
    }

    pub fn table(self) -> &'static [(&'static str, FieldClass)] {
        match self {
            EntityKind::Host => HOST_FIELDS,
            EntityKind::Group => GROUP_FIELDS,
            EntityKind::Identity => IDENTITY_FIELDS,
        }
    }
}

pub fn classify(kind: EntityKind, field: &str) -> Option<FieldClass> {
    kind.table()
        .iter()
        .find(|(n, _)| *n == field)
        .map(|(_, c)| *c)
}

/// Does a patch (as `(name, value)` pairs) touch a dangerous field? A field
/// missing from the table counts as dangerous, never as safe.
pub fn touches_dangerous(kind: EntityKind, fields: &[(&'static str, Value)]) -> bool {
    fields
        .iter()
        .any(|(n, _)| classify(kind, n) != Some(FieldClass::Safe))
}

pub fn body_tier(body: &OpBody) -> OpTier {
    use super::canonical::{group_field_values, host_field_values, identity_field_values};
    use super::op::DeviceRole;
    match body {
        OpBody::HostUpsert { fields, .. } => {
            upsert_tier(EntityKind::Host, &host_field_values(fields))
        }
        OpBody::GroupUpsert { fields, .. } => {
            upsert_tier(EntityKind::Group, &group_field_values(fields))
        }
        OpBody::IdentityUpsert { fields, .. } => {
            upsert_tier(EntityKind::Identity, &identity_field_values(fields))
        }
        OpBody::HostDelete { .. }
        | OpBody::GroupDelete { .. }
        | OpBody::IdentityDelete { .. }
        | OpBody::CredUpdate { .. }
        | OpBody::ShareRelease { .. }
        | OpBody::CertRevoke { .. }
        | OpBody::PhoneReenrollCancel { .. } => OpTier::Yellow,
        OpBody::DeviceAdd { role, .. } => match role {
            DeviceRole::Holder => OpTier::Red,
            DeviceRole::Mirror => OpTier::Yellow,
        },
        OpBody::DeviceRevoke { .. }
        | OpBody::CertIssue { .. }
        | OpBody::PhoneHandover { .. }
        | OpBody::PhoneReenroll { .. }
        | OpBody::RecoveryApprove { .. }
        | OpBody::FactorRegister { .. }
        | OpBody::DeviceRoleChange { .. }
        | OpBody::ExemptionSet { .. }
        | OpBody::BulkExport { .. }
        | OpBody::ModeChange { .. } => OpTier::Red,
        // Cryptographic data only: it changes nothing a user could be tricked
        // into trusting, and the ring ceremony around it is Red.
        OpBody::CertHostOpened { .. }
        | OpBody::RecoveryAck { .. }
        | OpBody::HeadAttest { .. }
        | OpBody::ShareGrant { .. }
        | OpBody::CertExtend { .. }
        | OpBody::CertRenew { .. }
        | OpBody::RedApprove { .. }
        | OpBody::RedPhoneSign { .. }
        | OpBody::RedFactor { .. }
        | OpBody::CheckpointPropose { .. }
        | OpBody::CheckpointSign { .. }
        | OpBody::WrapKeyRotate { .. }
        | OpBody::ShareReshareSession { .. }
        | OpBody::ShareStorageKey { .. }
        | OpBody::ShareReshareBatch { .. }
        | OpBody::ShareReshareConfirmBatch { .. } => OpTier::Green,
    }
}

fn upsert_tier(kind: EntityKind, fields: &[(&'static str, Value)]) -> OpTier {
    if touches_dangerous(kind, fields) {
        OpTier::Yellow
    } else {
        OpTier::Green
    }
}
