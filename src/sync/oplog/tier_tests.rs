//! Tests of `oplog::tier` that need the store's row types (`ManagedHost`, `HostGroup`, `Identity`);
//! the table itself lives in `sshub-sync-proto`.

use crate::store::{HostGroup, Identity, ManagedHost};
use serde_json::Value;
use sshub_sync_proto::oplog::tier::*;
use FieldClass::{Dangerous, LocalOnly, Safe};

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
