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

#[test]
fn a_write_whose_recording_fails_still_succeeds_and_the_next_publish_repairs_it() {
    let store = LauncherStore::open_in_memory().unwrap();
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
    install_locked(&store);
    // The identity is locked: recording cannot sign, but the caller must see success.
    let host = store.create_host(&NewHost::launcher("web", "10.0.0.1"));
    assert!(host.is_ok(), "{host:?}");
    assert_eq!(
        store.list_hosts().unwrap().len(),
        1,
        "exactly one host, no retry duplicates it"
    );
    assert_eq!(ops(&store, "HostUpsert").len(), 0);
    let warning = store
        .take_sync_warning()
        .expect("the failure is kept for the caller");
    assert!(warning.contains("could not be recorded"), "{warning}");
    assert!(store.take_sync_warning().is_none());
    // The key is available again at the next round: exactly one op, once.
    publish_pending(&store, &id).unwrap();
    assert_eq!(ops(&store, "HostUpsert").len(), 1);
    publish_pending(&store, &id).unwrap();
    assert_eq!(ops(&store, "HostUpsert").len(), 1);
}

// ======================================================== review round 2 (capture): N4 N5 N7 N8 N9 S1 S2

fn id_node() -> (Node, Arc<DeviceIdentity>) {
    let id = Arc::new(DeviceIdentity::generate());
    let store = LauncherStore::open_in_memory().unwrap();
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
    install_with_identity(&store, id.clone()).unwrap();
    (Node { store }, id)
}

#[test]
fn a_local_edit_does_not_clear_a_reference_the_projection_could_not_resolve() {
    let ((a, aid), (b, bid)) = (id_node(), id_node());
    for (x, y) in [(&a, &bid), (&b, &aid)] {
        x.store
            .sync_register_device(
                &y.public(),
                "peer",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
    }
    let ident = a
        .store
        .create_identity(&NewIdentity {
            name: "ci".into(),
            username: Some("u".into()),
            ..Default::default()
        })
        .unwrap();
    let host = a
        .store
        .create_host(&NewHost::launcher("h", "h.example"))
        .unwrap();
    a.store
        .update_host(
            host.id,
            &HostUpdate {
                identity_id: Some(Some(ident.id)),
                ..Default::default()
            },
        )
        .unwrap();
    pull(&b.store, &a.store);
    // b approves the host (and its identity reference) but not the identity itself.
    for op in b.store.sync_pending().unwrap() {
        if matches!(op.body, OpBody::HostUpsert { .. }) {
            b.store.sync_resolve_pending(&op.hash(), true).unwrap();
        }
    }
    let bh = b
        .store
        .list_hosts()
        .unwrap()
        .into_iter()
        .find(|h| h.name == "h")
        .expect("the approved host exists on b");
    b.store
        .update_host(
            bh.id,
            &HostUpdate {
                notes: Some(Some("only a note".into())),
                ..Default::default()
            },
        )
        .unwrap();
    let bdid = bid.public().device_id();
    let own: Vec<Op> = ops(&b.store, "HostUpsert")
        .into_iter()
        .filter(|o| o.device_id == bdid)
        .collect();
    let f = host_fields(own.last().expect("b recorded its edit"));
    assert_eq!(
        (f.notes.clone(), f.identity_uid),
        (Some(Some("only a note".into())), None),
        "the edit of a note also says something about the identity: {f:?}"
    );
}

#[test]
fn one_field_the_op_validator_refuses_does_not_block_later_captures_of_that_host() {
    let (n, _) = id_node();
    let host = n
        .store
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    let _ = n.store.update_host(
        host.id,
        &HostUpdate {
            notes: Some(Some("n".repeat(5000))),
            ..Default::default()
        },
    );
    n.store
        .update_host(
            host.id,
            &HostUpdate {
                label: Some(Some("x".into())),
                ..Default::default()
            },
        )
        .unwrap();
    let uid = n.store.host_uid(host.id).unwrap().unwrap();
    let view = n
        .store
        .with_conn(|c| crate::sync::apply::effective_view(c, EntityKind::Host, &uid))
        .unwrap();
    assert_eq!(
        view.fields.get("label"),
        Some(&Value::String("x".into())),
        "a later, valid edit never reached the ring"
    );
}

#[test]
fn publish_pending_publishes_an_edit_that_missed_the_ring() {
    let (n, id) = id_node();
    let host = n
        .store
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    // The recording of this edit failed (a locked key, a full disk): the row is saved, no op.
    n.store
        .with_conn(|c| {
            crate::store::update_host_on(
                c,
                host.id,
                &HostUpdate {
                    notes: Some(Some("edited".into())),
                    ..Default::default()
                },
            )?;
            Ok(())
        })
        .unwrap();
    publish_pending(&n.store, &id).unwrap();
    assert_eq!(
        ops(&n.store, "HostUpsert").len(),
        2,
        "the unrecorded edit was not published by the next sync round"
    );
}

#[test]
fn one_unpublishable_host_does_not_stop_the_sync_round_or_the_hosts_after_it() {
    let (n, id) = id_node();
    n.store
        .with_conn(|c| {
            crate::store::create_host_on(c, &NewHost::launcher("bad\u{7}name", "10.0.0.9"))?;
            crate::store::create_host_on(c, &NewHost::launcher("good", "10.0.0.8"))?;
            Ok(())
        })
        .unwrap();
    let r = publish_pending(&n.store, &id);
    assert!(
        r.is_ok() && !ops(&n.store, "HostUpsert").is_empty(),
        "publish_pending: {:?}; ops recorded: {}",
        r.as_ref().map_err(|e| format!("{e:#}")),
        ops(&n.store, "HostUpsert").len()
    );
}

#[test]
fn a_note_edit_on_a_synced_in_host_does_not_pin_its_port() {
    let ((a, aid), (b, bid)) = (id_node(), id_node());
    for (x, y) in [(&a, &bid), (&b, &aid)] {
        x.store
            .sync_register_device(
                &y.public(),
                "peer",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
    }
    // The ring's host names no port: it inherits its group's or ssh's default.
    a.store
        .sync_append_local(
            &aid,
            OpBody::HostUpsert {
                host_uid: [9; 16],
                fields: crate::sync::oplog::op::HostFields {
                    name: Some("h".into()),
                    address: Some("h.example".into()),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    pull(&b.store, &a.store);
    approve_all(&b.store);
    let bh = b
        .store
        .list_hosts()
        .unwrap()
        .into_iter()
        .find(|h| h.name == "h")
        .expect("the host reached b");
    b.store
        .update_host(
            bh.id,
            &HostUpdate {
                notes: Some(Some("a note".into())),
                ..Default::default()
            },
        )
        .unwrap();
    let bdid = bid.public().device_id();
    let own: Vec<Op> = ops(&b.store, "HostUpsert")
        .into_iter()
        .filter(|o| o.device_id == bdid)
        .collect();
    let f = host_fields(own.last().expect("b recorded its edit"));
    assert_eq!(
        (f.notes.clone(), f.port),
        (Some(Some("a note".into())), None),
        "a note edit also sets the port (a Yellow field for every other device): {f:?}"
    );
}

#[test]
fn a_refused_field_is_reported_and_the_rest_of_the_edit_still_reaches_the_ring() {
    let (n, _) = id_node();
    let host = n
        .store
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    n.store
        .update_host(
            host.id,
            &HostUpdate {
                notes: Some(Some("n".repeat(5000))),
                label: Some(Some("fine".into())),
                ..Default::default()
            },
        )
        .unwrap();
    let warning = n.store.take_sync_warning().expect("the user is told");
    assert!(warning.contains("notes"), "{warning}");
    let last = ops(&n.store, "HostUpsert").pop().unwrap();
    let f = host_fields(&last);
    assert_eq!(f.label, Some(Some("fine".into())));
    assert_eq!(f.notes, None, "the refused note is not in the op");
}

#[test]
fn the_device_key_is_loaded_before_the_store_connection_is_taken() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Probe {
        store: Arc<LauncherStore>,
        ran: Arc<AtomicBool>,
    }
    impl LocalChangeSink for Probe {
        fn prepare(&self) -> Result<()> {
            // Taking the connection here would deadlock if the caller still held it.
            self.store.with_conn(|_| Ok(()))?;
            self.ran.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn changed(&self, _: &Connection, _: ChangeScope) -> Result<()> {
            Ok(())
        }
    }
    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    store
        .sync_register_device(
            &DeviceIdentity::generate().public(),
            "me",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    let ran = Arc::new(AtomicBool::new(false));
    store.set_local_change_sink(Box::new(Probe {
        store: store.clone(),
        ran: ran.clone(),
    }));
    let (tx, rx) = std::sync::mpsc::channel();
    let s = store.clone();
    std::thread::spawn(move || {
        let _ = s.create_host(&NewHost::launcher("web", "10.0.0.1"));
        let _ = tx.send(());
    });
    rx.recv_timeout(std::time::Duration::from_secs(20))
        .expect("prepare ran under the connection lock: deadlock");
    assert!(ran.load(Ordering::SeqCst));
}

#[test]
fn a_profile_in_a_ring_without_its_device_key_never_gets_a_new_identity() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ProfilePaths {
        data_root: dir.path().to_path_buf(),
        id: "t".into(),
        name: "t".into(),
        root: dir.path().to_path_buf(),
        config_file: dir.path().join("config.toml"),
        ssh_config: dir.path().join("ssh_config"),
        compat: false,
    };
    let store = LauncherStore::open_in_memory().unwrap();
    store
        .sync_register_device(
            &DeviceIdentity::generate().public(),
            "someone",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    install(&store, &paths, |_| Box::new(MemoryPasswordStore::default())).unwrap();
    let devices_before = store.sync_devices().unwrap().len();
    store
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    let warning = store.take_sync_warning().expect("refused with a warning");
    assert!(warning.contains("device key"), "{warning}");
    assert_eq!(
        store.sync_devices().unwrap().len(),
        devices_before,
        "no new holder"
    );
    assert!(!crate::sync::identity_store::key_path(&paths).exists());
    assert_eq!(ops(&store, "HostUpsert").len(), 0);
}

/// Two nodes of one ring; `a` authored host `h` (no port, no transport) and `b` has approved it.
fn synced_in_host() -> (Node, Arc<DeviceIdentity>, i64) {
    let ((a, aid), (b, bid)) = (id_node(), id_node());
    for (x, y) in [(&a, &bid), (&b, &aid)] {
        x.store
            .sync_register_device(
                &y.public(),
                "peer",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
    }
    a.store
        .sync_append_local(
            &aid,
            OpBody::HostUpsert {
                host_uid: [9; 16],
                fields: crate::sync::oplog::op::HostFields {
                    name: Some("h".into()),
                    address: Some("h.example".into()),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    pull(&b.store, &a.store);
    approve_all(&b.store);
    let id = b
        .store
        .list_hosts()
        .unwrap()
        .into_iter()
        .find(|h| h.name == "h")
        .expect("the host reached b")
        .id;
    (b, bid, id)
}

#[test]
fn a_host_the_ring_gave_no_port_or_transport_stores_null_so_they_inherit() {
    let (b, _, id) = synced_in_host();
    let h = b.store.get_host(id).unwrap().unwrap();
    assert_eq!((h.port, h.transport), (None, None));
}

#[test]
fn an_explicit_port_22_or_ssh_on_a_synced_in_host_is_published_and_a_note_edit_alone_is_not() {
    let (b, bid, id) = synced_in_host();
    let mine = |b: &Node| -> Vec<Op> {
        ops(&b.store, "HostUpsert")
            .into_iter()
            .filter(|o| o.device_id == bid.public().device_id())
            .collect()
    };
    // The user pins port 22 and ssh on purpose (the group default on the other devices differs).
    b.store
        .update_host(
            id,
            &HostUpdate {
                port: Some(Some(22)),
                transport: Some(Some(crate::session_transport::SessionTransport::Ssh)),
                ..Default::default()
            },
        )
        .unwrap();
    let f = host_fields(
        mine(&b)
            .last()
            .expect("the explicit choice reached the log"),
    );
    assert_eq!(f.port, Some(Some(22)));
    assert_eq!(f.transport, Some(Some("ssh".into())));
    // A later note edit says nothing about either: both now equal the view.
    let before = mine(&b).len();
    b.store
        .update_host(
            id,
            &HostUpdate {
                notes: Some(Some("a note".into())),
                ..Default::default()
            },
        )
        .unwrap();
    let all = mine(&b);
    assert_eq!(all.len(), before + 1);
    let f = host_fields(all.last().unwrap());
    assert_eq!(
        (f.notes.clone(), f.port, f.transport),
        (Some(Some("a note".into())), None, None)
    );
}

#[test]
fn a_new_local_host_with_the_form_defaults_does_not_pin_port_or_transport() {
    let (n, _) = id_node();
    n.store
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    let f = host_fields(ops(&n.store, "HostUpsert").last().unwrap());
    assert_eq!((f.port, f.transport), (None, None));
}
