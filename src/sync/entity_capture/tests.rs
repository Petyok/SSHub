use std::collections::HashSet;

use super::*;
use crate::app::tests::MemoryPasswordStore;
use crate::store::transfer::apply_transfer_plan_with_secrets;
use crate::store::{
    build_transfer_plan, DestIndex, GroupTransferMode, HostUpdate, NewHost, NewHostGroup,
    NewIdentity, TransferMode, TransferSelection, TransferSnapshot,
};
use crate::sync::oplog::{Attendance, DeviceKind, DeviceRole, Op, VerifiedBy};

struct Node {
    store: LauncherStore,
}

/// `n` stores that know each other as holders, each with the sink installed.
fn ring(n: usize) -> Vec<Node> {
    let ids: Vec<Arc<DeviceIdentity>> = (0..n)
        .map(|_| Arc::new(DeviceIdentity::generate()))
        .collect();
    ids.iter()
        .map(|me| {
            let store = LauncherStore::open_in_memory().unwrap();
            for other in &ids {
                store
                    .sync_register_device(
                        &other.public(),
                        "peer",
                        DeviceRole::Holder,
                        Attendance::Attended,
                        DeviceKind::Terminal,
                        VerifiedBy::Sas,
                    )
                    .unwrap();
            }
            install_with_identity(&store, me.clone()).unwrap();
            Node { store }
        })
        .collect()
}

/// A profile with no ring and the sink installed.
fn solo() -> LauncherStore {
    let store = LauncherStore::open_in_memory().unwrap();
    install_with_identity(&store, Arc::new(DeviceIdentity::generate())).unwrap();
    store
}

fn ops(store: &LauncherStore, kind: &str) -> Vec<Op> {
    crate::sync::quorum::ops_of(store, kind, None).unwrap()
}

fn count(store: &LauncherStore, sql: &str) -> i64 {
    store
        .with_conn(|c| Ok(c.query_row(sql, [], |r| r.get(0))?))
        .unwrap()
}

fn all_ops(store: &LauncherStore) -> i64 {
    count(store, "SELECT COUNT(*) FROM sync_ops")
}

fn local_ops(store: &LauncherStore) -> i64 {
    count(store, "SELECT COUNT(*) FROM sync_ops WHERE local = 1")
}

fn pull(to: &LauncherStore, from: &LauncherStore) {
    for (dev, _, _) in from.sync_chain_heads().unwrap() {
        let have = to
            .sync_chain_heads()
            .unwrap()
            .into_iter()
            .find(|(d, _, _)| *d == dev)
            .map_or(0, |(_, s, _)| s);
        for op in from.sync_chain_tail(&dev, have).unwrap() {
            to.sync_ingest(op).unwrap();
        }
    }
}

fn approve_all(store: &LauncherStore) {
    while let Some(op) = store.sync_pending().unwrap().into_iter().next() {
        store.sync_resolve_pending(&op.hash(), true).unwrap();
    }
}

fn host_fields(op: &Op) -> crate::sync::oplog::op::HostFields {
    match &op.body {
        OpBody::HostUpsert { fields, .. } => fields.clone(),
        other => panic!("not a host upsert: {other:?}"),
    }
}

#[test]
fn a_local_create_appends_one_op_and_an_edit_only_the_fields_that_changed() {
    let n = &ring(1)[0];
    let host = n
        .store
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    let created = ops(&n.store, "HostUpsert");
    assert_eq!(created.len(), 1);
    let f = host_fields(&created[0]);
    assert_eq!(
        (f.name.as_deref(), f.address.as_deref()),
        (Some("web"), Some("10.0.0.1"))
    );
    assert_eq!(f.notes, None);

    let note = HostUpdate {
        notes: Some(Some("n".into())),
        ..Default::default()
    };
    n.store.update_host(host.id, &note).unwrap();
    let after = ops(&n.store, "HostUpsert");
    assert_eq!(after.len(), 2);
    let f = host_fields(&after[1]);
    assert_eq!(f.notes, Some(Some("n".into())));
    assert_eq!((f.name, f.address, f.port), (None, None, None));
    // Saving the same thing again says nothing.
    n.store.update_host(host.id, &note).unwrap();
    assert_eq!(ops(&n.store, "HostUpsert").len(), 2);
}

#[test]
fn groups_identities_memberships_and_favorites_are_captured() {
    let n = &ring(1)[0];
    let group = n
        .store
        .create_group(&NewHostGroup {
            name: "prod".into(),
            ..Default::default()
        })
        .unwrap();
    let ident = n
        .store
        .create_identity(&NewIdentity {
            name: "deploy".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(ops(&n.store, "GroupUpsert").len(), 1);
    assert_eq!(ops(&n.store, "IdentityUpsert").len(), 1);
    let host = n
        .store
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    n.store.set_host_groups(host.id, &[group.id]).unwrap();
    let f = host_fields(ops(&n.store, "HostUpsert").last().unwrap());
    assert_eq!(f.group_uids.as_ref().map(Vec::len), Some(1));
    assert_eq!(f.name, None);
    n.store
        .update_host(
            host.id,
            &HostUpdate {
                favorite: Some(true),
                identity_id: Some(Some(ident.id)),
                ..Default::default()
            },
        )
        .unwrap();
    let f = host_fields(ops(&n.store, "HostUpsert").last().unwrap());
    assert_eq!(f.favorite, Some(true));
    assert!(matches!(f.identity_uid, Some(Some(_))));
}

#[test]
fn deleting_a_host_a_group_or_an_identity_appends_a_delete() {
    let n = &ring(1)[0];
    let host = n
        .store
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    let group = n
        .store
        .create_group(&NewHostGroup {
            name: "g".into(),
            ..Default::default()
        })
        .unwrap();
    let ident = n
        .store
        .create_identity(&NewIdentity {
            name: "i".into(),
            ..Default::default()
        })
        .unwrap();
    n.store.delete_host(host.id).unwrap();
    n.store.delete_group(group.id).unwrap();
    n.store.delete_identity(ident.id).unwrap();
    assert_eq!(ops(&n.store, "HostDelete").len(), 1);
    assert_eq!(ops(&n.store, "GroupDelete").len(), 1);
    assert_eq!(ops(&n.store, "IdentityDelete").len(), 1);
    // A delete of something the log never had says nothing.
    let local = n
        .store
        .create_host(&NewHost::launcher("tmp", "10.0.0.2"))
        .unwrap();
    n.store.delete_host(local.id).unwrap();
    assert_eq!(ops(&n.store, "HostDelete").len(), 2);
}

#[test]
fn a_profile_without_a_ring_appends_nothing() {
    let store = solo();
    let host = store
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    store
        .update_host(
            host.id,
            &HostUpdate {
                notes: Some(Some("n".into())),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .create_group(&NewHostGroup {
            name: "g".into(),
            ..Default::default()
        })
        .unwrap();
    store.delete_host(host.id).unwrap();
    assert_eq!(all_ops(&store), 0);
    assert!(
        store.sync_devices().unwrap().is_empty(),
        "no ring was founded by a local edit"
    );
}

#[test]
fn ssh_config_hosts_are_the_machines_own_and_never_replicate() {
    use crate::store::SshConfigHostImport;
    let n = &ring(1)[0];
    n.store
        .upsert_ssh_config_host(&SshConfigHostImport {
            name: "cfg".into(),
            address: "10.9.9.9".into(),
            port: 22,
            proxy_jump: None,
            forward_agent: false,
            remote_command: None,
            transport: Default::default(),
            ssh_config_hash: "h".into(),
            tags: vec![],
            notes: None,
            environment: None,
            favorite: false,
            last_connected: None,
            session_logging: Default::default(),
            ping: Default::default(),
        })
        .unwrap();
    assert_eq!(all_ops(&n.store), 0);
}

#[test]
fn applying_a_remote_op_appends_none_and_two_stores_converge_on_the_host() {
    let r = ring(2);
    let (a, b) = (&r[0].store, &r[1].store);
    let host = a
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    assert_eq!(local_ops(a), 1);
    pull(b, a);
    approve_all(b);
    let got = b
        .get_host_by_name("web")
        .unwrap()
        .expect("the host reached b");
    assert_eq!(got.address, "10.0.0.1");
    assert_eq!(local_ops(b), 0, "applying a synced op must not echo");
    // And back: an edit on b is one op, applied on a without a second one.
    b.update_host(
        got.id,
        &HostUpdate {
            notes: Some(Some("hello".into())),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(local_ops(b), 1);
    pull(a, b);
    approve_all(a);
    assert_eq!(
        a.get_host(host.id).unwrap().unwrap().notes.as_deref(),
        Some("hello")
    );
    assert_eq!(local_ops(a), 1);
    assert_eq!(
        a.sync_projection_hash().unwrap(),
        b.sync_projection_hash().unwrap()
    );
}

#[test]
fn a_dangerous_field_from_a_peer_waits_and_the_review_names_it() {
    let r = ring(2);
    let (a, b) = (&r[0].store, &r[1].store);
    let host = a
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    pull(b, a);
    approve_all(b);
    a.update_host(
        host.id,
        &HostUpdate {
            address: Some("evil.example".into()),
            ..Default::default()
        },
    )
    .unwrap();
    pull(b, a);
    let pending = b.sync_pending_with_reasons().unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].1, "tier");
    assert_eq!(
        b.get_host_by_name("web").unwrap().unwrap().address,
        "10.0.0.1"
    );
    let devices = b.sync_devices().unwrap();
    let view = crate::app::sync_view::pending_view(b, &pending[0].0, &devices, Some("tier".into()));
    let change = view
        .changes
        .iter()
        .find(|c| c.field == "address")
        .unwrap_or_else(|| panic!("no address change in {view:?}"));
    assert!(change.dangerous);
    assert_eq!(
        (change.from.as_str(), change.to.as_str()),
        ("10.0.0.1", "evil.example")
    );
}

#[test]
fn a_suffixed_name_shown_for_a_clash_is_not_a_rename() {
    let r = ring(2);
    let (a, b) = (&r[0].store, &r[1].store);
    a.create_host(&NewHost::launcher("dup", "10.0.0.1"))
        .unwrap();
    b.create_host(&NewHost::launcher("dup", "10.0.0.2"))
        .unwrap();
    pull(b, a);
    approve_all(b);
    pull(a, b);
    approve_all(a);
    for store in [a, b] {
        let suffixed = store
            .list_hosts()
            .unwrap()
            .into_iter()
            .find(|h| h.name.starts_with("dup ("))
            .expect("one of the two shows a suffix");
        let before = ops(store, "HostUpsert").len();
        store
            .update_host(
                suffixed.id,
                &HostUpdate {
                    notes: Some(Some("x".into())),
                    ..Default::default()
                },
            )
            .unwrap();
        let all = ops(store, "HostUpsert");
        assert_eq!(all.len(), before + 1);
        assert_eq!(host_fields(all.last().unwrap()).name, None);
    }
}

#[test]
fn imports_produce_one_op_per_entity() {
    let n = &ring(1)[0];
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("L00t.csv"),
        "Label,Host,Port,Username,Password,SSH_Key,OS\nweb1,10.0.0.1,22,root,,,\nweb2,10.0.0.2,22,root,,,\nweb3,10.0.0.3,2222,admin,,,\n",
    )
    .unwrap();
    let report = crate::import::termius_csv::import_csv_export(
        dir.path(),
        &n.store,
        &MemoryPasswordStore::default(),
    )
    .unwrap();
    assert_eq!(report.hosts_imported, 3, "{report:?}");
    assert_eq!(ops(&n.store, "HostUpsert").len(), 3);
    // A bulk of plain creates is one op each.
    for i in 0..25 {
        n.store
            .create_host(&NewHost::launcher(format!("bulk{i}"), "10.1.0.1"))
            .unwrap();
    }
    assert_eq!(ops(&n.store, "HostUpsert").len(), 28);
}

#[test]
fn a_host_transferred_into_a_ringed_profile_replicates_there_too() {
    let mut src = LauncherStore::open_in_memory().unwrap();
    src.create_host(&NewHost::launcher("web", "web.example.com"))
        .unwrap();
    // `dest` is a ringed profile.
    let mut dest = LauncherStore::open_in_memory().unwrap();
    let ids: Vec<Arc<DeviceIdentity>> = (0..2)
        .map(|_| Arc::new(DeviceIdentity::generate()))
        .collect();
    for id in &ids {
        dest.sync_register_device(
            &id.public(),
            "p",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    }
    install_with_identity(&dest, ids[0].clone()).unwrap();
    let plan = build_transfer_plan(
        &TransferSnapshot::capture(&src).unwrap(),
        &DestIndex::capture(&dest, "dest").unwrap(),
        &TransferSelection {
            host_names: vec!["web".into()],
            ..Default::default()
        },
        TransferMode::Copy,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    apply_transfer_plan_with_secrets(&mut src, &mut dest, &plan, TransferMode::Copy, None).unwrap();
    assert_eq!(dest.list_hosts().unwrap().len(), 1);
    assert_eq!(
        ops(&dest, "HostUpsert").len(),
        1,
        "the transferred host is announced to the ring"
    );
    // A move also tells the SOURCE ring about the deletion.
    let mut src2 = ring(1).remove(0).store;
    src2.create_host(&NewHost::launcher("db", "db.example.com"))
        .unwrap();
    let mut dest2 = LauncherStore::open_in_memory().unwrap();
    let plan = build_transfer_plan(
        &TransferSnapshot::capture(&src2).unwrap(),
        &DestIndex::capture(&dest2, "d2").unwrap(),
        &TransferSelection {
            host_names: vec!["db".into()],
            ..Default::default()
        },
        TransferMode::Move,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    apply_transfer_plan_with_secrets(&mut src2, &mut dest2, &plan, TransferMode::Move, None)
        .unwrap();
    assert!(src2.list_hosts().unwrap().is_empty());
    assert_eq!(ops(&src2, "HostDelete").len(), 1);
}

#[test]
fn what_predates_the_ring_is_published_when_the_sink_is_installed() {
    let store = LauncherStore::open_in_memory().unwrap();
    store
        .create_host(&NewHost::launcher("old", "10.0.0.5"))
        .unwrap();
    let id = Arc::new(DeviceIdentity::generate());
    store
        .sync_register_device(
            &id.public(),
            "me",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    assert_eq!(ops(&store, "HostUpsert").len(), 0);
    publish_pending(&store, &id).unwrap();
    assert_eq!(ops(&store, "HostUpsert").len(), 1);
    // Idempotent.
    publish_pending(&store, &id).unwrap();
    assert_eq!(ops(&store, "HostUpsert").len(), 1);
}
