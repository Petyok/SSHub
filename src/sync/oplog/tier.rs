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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{HostGroup, Identity, ManagedHost};

    fn names<'a>(table: &[(&'a str, FieldClass)]) -> Vec<&'a str> {
        table.iter().map(|(n, _)| *n).collect()
    }

    /// Compile-time exhaustive: adding a field to `ManagedHost`, `HostGroup` or
    /// `Identity` breaks these destructurings, and a field missing from the
    /// table fails the assertions.
    #[test]
    fn every_store_field_is_classified() {
        fn host(h: ManagedHost) -> Vec<&'static str> {
            let ManagedHost {
                id: _,
                name: _,
                label: _,
                address: _,
                port: _,
                group_id: _,
                identity_id: _,
                group: _,
                groups: _,
                identity: _,
                os_icon: _,
                tags: _,
                notes: _,
                proxy_jump: _,
                forward_agent: _,
                remote_command: _,
                environment: _,
                sort_order: _,
                favorite: _,
                last_connected: _,
                source: _,
                ssh_config_hash: _,
                has_password: _,
                secret_lookup: _,
                username: _,
                session_logging: _,
                ping: _,
                transport: _,
                created_at: _,
                updated_at: _,
            } = h;
            vec![
                "id",
                "name",
                "label",
                "address",
                "port",
                "group_id",
                "identity_id",
                "group",
                "groups",
                "identity",
                "os_icon",
                "tags",
                "notes",
                "proxy_jump",
                "forward_agent",
                "remote_command",
                "environment",
                "sort_order",
                "favorite",
                "last_connected",
                "source",
                "ssh_config_hash",
                "has_password",
                "secret_lookup",
                "username",
                "session_logging",
                "ping",
                "transport",
                "created_at",
                "updated_at",
            ]
        }
        fn group(g: HostGroup) -> Vec<&'static str> {
            let HostGroup {
                id: _,
                name: _,
                sort_order: _,
                default_identity_id: _,
                default_username: _,
                default_port: _,
                default_proxy_jump: _,
                default_transport: _,
                default_forward_agent: _,
                parent_id: _,
                reserved: _,
            } = g;
            vec![
                "id",
                "name",
                "sort_order",
                "default_identity_id",
                "default_username",
                "default_port",
                "default_proxy_jump",
                "default_transport",
                "default_forward_agent",
                "parent_id",
                "reserved",
            ]
        }
        fn identity(i: Identity) -> Vec<&'static str> {
            let Identity {
                id: _,
                name: _,
                username: _,
                private_key: _,
                certificate: _,
                has_password: _,
            } = i;
            vec![
                "id",
                "name",
                "username",
                "private_key",
                "certificate",
                "has_password",
            ]
        }
        // The functions above are only here to be compiled; the lists are what
        // they destructure, kept next to the patterns so both change together.
        let _ = (host, group, identity);
        let host_names: &[&str] = &[
            "id",
            "name",
            "label",
            "address",
            "port",
            "group_id",
            "identity_id",
            "group",
            "groups",
            "identity",
            "os_icon",
            "tags",
            "notes",
            "proxy_jump",
            "forward_agent",
            "remote_command",
            "environment",
            "sort_order",
            "favorite",
            "last_connected",
            "source",
            "ssh_config_hash",
            "has_password",
            "secret_lookup",
            "username",
            "session_logging",
            "ping",
            "transport",
            "created_at",
            "updated_at",
        ];
        for n in host_names {
            assert!(
                classify(EntityKind::Host, n).is_some(),
                "host field {n} unclassified"
            );
        }
        for n in [
            "id",
            "name",
            "sort_order",
            "default_identity_id",
            "default_username",
            "default_port",
            "default_proxy_jump",
            "default_transport",
            "default_forward_agent",
            "parent_id",
            "reserved",
        ] {
            assert!(
                classify(EntityKind::Group, n).is_some(),
                "group field {n} unclassified"
            );
        }
        for n in [
            "id",
            "name",
            "username",
            "private_key",
            "certificate",
            "has_password",
        ] {
            assert!(
                classify(EntityKind::Identity, n).is_some(),
                "identity field {n} unclassified"
            );
        }
    }

    #[test]
    fn tables_have_no_duplicates() {
        for t in [HOST_FIELDS, GROUP_FIELDS, IDENTITY_FIELDS] {
            let mut n = names(t);
            n.sort_unstable();
            n.dedup();
            assert_eq!(n.len(), t.len());
        }
    }

    #[test]
    fn contract_c5_sets() {
        for n in ["name", "label", "notes", "tags", "os_icon", "favorite"] {
            assert_eq!(classify(EntityKind::Host, n), Some(Safe), "{n}");
        }
        for n in [
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
        ] {
            assert_eq!(classify(EntityKind::Host, n), Some(Dangerous), "{n}");
        }
        for n in [
            "default_proxy_jump",
            "default_identity_uid",
            "default_username",
            "default_port",
            "default_transport",
            "default_forward_agent",
            "parent_uid",
        ] {
            assert_eq!(classify(EntityKind::Group, n), Some(Dangerous), "{n}");
        }
        for n in [
            "id",
            "created_at",
            "updated_at",
            "last_connected",
            "sort_order",
            "ping",
            "session_logging",
            "ssh_config_hash",
            "source",
        ] {
            assert_eq!(classify(EntityKind::Host, n), Some(LocalOnly), "{n}");
        }
        assert_eq!(classify(EntityKind::Host, "never_heard_of_it"), None);
    }

    #[test]
    fn unknown_field_counts_as_dangerous() {
        assert!(touches_dangerous(
            EntityKind::Host,
            &[("never_heard_of_it", Value::Null)]
        ));
        assert!(!touches_dangerous(
            EntityKind::Host,
            &[("name", Value::Null), ("notes", Value::Null)]
        ));
    }
}
