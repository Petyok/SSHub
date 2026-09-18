//! Cross-profile transfer engine: behavioral contract tests.
//!
//! These tests target ONLY the `store::transfer` contract API
//! (`TransferSnapshot` / `DestIndex` / `TransferSelection` /
//! `build_transfer_plan` / `apply_transfer_plan` / `unique_dest_name` /
//! `TransferMode` / `GroupTransferMode`). Written first (TDD): they fail on
//! current code because the `transfer` module does not exist yet.

use std::collections::HashSet;
use std::path::PathBuf;

use sshub::store::transfer::{
    apply_transfer_plan, build_transfer_plan, unique_dest_name, DestIndex, GroupTransferMode,
    TransferMode, TransferSelection, TransferSnapshot,
};
use sshub::store::{LauncherStore, NewHost, NewHostGroup, NewIdentity, NewTunnel, TunnelType};

fn mk_identity(store: &LauncherStore, name: &str) -> i64 {
    store
        .create_identity(&NewIdentity {
            name: name.into(),
            username: Some(format!("{name}-user")),
            private_key: Some(format!("/tmp/{name}.key").into()),
            certificate: None,
            sort_order: 0,
            has_password: false,
        })
        .unwrap()
        .id
}

fn mk_host(store: &LauncherStore, name: &str, identity_id: Option<i64>, group_id: Option<i64>) {
    store
        .create_host(&NewHost {
            name: name.into(),
            address: format!("{name}.example.com"),
            group_id,
            identity_id,
            ..NewHost::launcher(name, "")
        })
        .unwrap();
}

fn mk_group(store: &LauncherStore, name: &str) -> i64 {
    store
        .create_group(&NewHostGroup {
            name: name.into(),
            ..Default::default()
        })
        .unwrap()
        .id
}

fn mk_tunnel(store: &LauncherStore, host_id: i64, label: &str) {
    store
        .create_tunnel(&NewTunnel {
            host_id: Some(host_id),
            tunnel_type: TunnelType::Local,
            local_port: 8080,
            remote_host: "localhost".into(),
            remote_port: 80,
            label: Some(label.into()),
            auto_connect: false,
        })
        .unwrap();
}

fn snapshot(store: &LauncherStore) -> TransferSnapshot {
    TransferSnapshot::capture(store).unwrap()
}

fn dest_index(store: &LauncherStore) -> DestIndex {
    DestIndex::capture(store, "dest").unwrap()
}

fn host_selection(names: &[&str]) -> TransferSelection {
    TransferSelection {
        host_names: names.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn transfer_copy_host_with_identity() {
    let src = LauncherStore::open_in_memory().unwrap();
    let mut dest = LauncherStore::open_in_memory().unwrap();
    let id = mk_identity(&src, "id-a");
    mk_host(&src, "web", Some(id), None);

    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &host_selection(&["web"]),
        TransferMode::Copy,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    assert!(plan.blocked().is_empty());
    assert_eq!(plan.runnable().len(), 2); // host + auto-carried identity
                                          // Oracle anchor: the exact pre-apply source row; copy must leave it
                                          // byte-identical.
    let src_web_before = src.get_host_by_name("web").unwrap().unwrap();
    let mut src = src;
    let result = apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap();
    assert!(result.blocked.is_empty());
    assert_eq!(result.transferred, 2);
    assert!(result.renamed.is_empty());

    // Destination has copies wired together.
    let dest_host = dest.get_host_by_name("web").unwrap().expect("dest host");
    let dest_id = dest
        .get_identity_by_name("id-a")
        .unwrap()
        .expect("dest identity");
    assert_eq!(dest_host.identity_id, Some(dest_id.id));
    assert_eq!(dest_id.username.as_deref(), Some("id-a-user"));
    // Full-row carry: hand-built expected field values, not just linkage.
    assert_eq!(dest_host.address, "web.example.com");
    assert_eq!(dest_id.private_key, Some(PathBuf::from("/tmp/id-a.key")));
    // Source retains originals byte-identical (copy, not move).
    assert_eq!(
        src.get_host_by_name("web").unwrap().unwrap(),
        src_web_before
    );
    assert!(src.get_identity_by_name("id-a").unwrap().is_some());
}

#[test]
fn transfer_move_host_with_identity() {
    let mut src = LauncherStore::open_in_memory().unwrap();
    let mut dest = LauncherStore::open_in_memory().unwrap();
    let id = mk_identity(&src, "shared");
    mk_host(&src, "web", Some(id), None);
    mk_host(&src, "db", Some(id), None); // still references the identity

    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &host_selection(&["web"]),
        TransferMode::Move,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    let result = apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Move).unwrap();

    assert!(result.blocked.is_empty());
    // Moved host is gone from source, present in dest with a copied identity.
    assert!(src.get_host_by_name("web").unwrap().is_none());
    assert!(dest.get_host_by_name("web").unwrap().is_some());
    assert!(dest.get_identity_by_name("shared").unwrap().is_some());
    // The moved host must be wired to the *copied* identity in dest, and the
    // remaining source host must keep its original linkage untouched.
    let dest_shared_id = dest
        .get_identity_by_name("shared")
        .unwrap()
        .expect("dest identity")
        .id;
    assert_eq!(
        dest.get_host_by_name("web")
            .unwrap()
            .expect("dest host")
            .identity_id,
        Some(dest_shared_id)
    );
    assert_eq!(
        src.get_host_by_name("db")
            .unwrap()
            .expect("src host")
            .identity_id,
        Some(id)
    );
    // The carried identity is still referenced by the remaining host, so the
    // source row must survive the move.
    assert!(src.get_host_by_name("db").unwrap().is_some());
    assert!(src.get_identity_by_name("shared").unwrap().is_some());
}

#[test]
fn transfer_group_with_members() {
    let src = LauncherStore::open_in_memory().unwrap();
    let mut dest = LauncherStore::open_in_memory().unwrap();
    let gid = mk_group(&src, "ops");
    mk_host(&src, "web", None, Some(gid));
    mk_host(&src, "db", None, Some(gid));

    let selection = TransferSelection {
        group_names: vec!["ops".into()],
        ..Default::default()
    };
    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &selection,
        TransferMode::Copy,
        GroupTransferMode::WithHosts,
        &HashSet::new(),
    );

    let mut src = src;
    let result = apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap();
    assert!(result.blocked.is_empty());

    let dest_group = dest
        .list_groups()
        .unwrap()
        .into_iter()
        .find(|g| g.name == "ops")
        .expect("dest group");
    assert!(!dest_group.reserved);
    for name in ["web", "db"] {
        let h = dest.get_host_by_name(name).unwrap().expect("dest member");
        assert!(
            h.groups.iter().any(|g| g.name == "ops"),
            "{name} keeps its group membership in dest"
        );
    }
    assert_eq!(
        dest.get_host_by_name("web")
            .unwrap()
            .expect("dest member")
            .address,
        "web.example.com"
    );
    // Source retains everything on copy.
    assert!(src.get_host_by_name("web").unwrap().is_some());
    // Copy must not strip the source: members keep their membership at home,
    // and the carried rows arrive with full field values.
    for name in ["web", "db"] {
        let h = src.get_host_by_name(name).unwrap().expect("src member");
        assert!(
            h.groups.iter().any(|g| g.name == "ops"),
            "{name} keeps its group membership in source"
        );
        assert_eq!(h.address, format!("{name}.example.com"));
    }
}

#[test]
fn transfer_tunnel_carry() {
    let src = LauncherStore::open_in_memory().unwrap();
    let mut dest = LauncherStore::open_in_memory().unwrap();
    mk_host(&src, "web", None, None);
    let host_id = src.get_host_by_name("web").unwrap().unwrap().id;
    mk_tunnel(&src, host_id, "t1");

    // Only the host is selected; its tunnel must be auto-carried.
    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &host_selection(&["web"]),
        TransferMode::Copy,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    assert!(plan.items.iter().any(|i| i.kind == "tunnel"));

    let mut src = src;
    let result = apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap();
    assert!(result.blocked.is_empty());

    let tunnels = dest.list_tunnels().unwrap();
    assert_eq!(tunnels.len(), 1);
    let dest_host_id = dest.get_host_by_name("web").unwrap().unwrap().id;
    assert_eq!(tunnels[0].host_id, Some(dest_host_id));
    assert_eq!(tunnels[0].label.as_deref(), Some("t1"));
    // Full tunnel row carry: endpoints remapped to the copied host, ports kept.
    assert_eq!(tunnels[0].local_port, 8080);
    assert_eq!(tunnels[0].remote_host, "localhost");
    assert_eq!(tunnels[0].remote_port, 80);
}

#[test]
fn transfer_auto_rename() {
    // unique_dest_name never overwrites: first free " (copy N)" wins.
    assert_eq!(unique_dest_name("web", &[]), "web");
    assert_eq!(unique_dest_name("web", &["web".into()]), "web (copy 1)");
    assert_eq!(
        unique_dest_name("web", &["web".into(), "web (copy 1)".into()]),
        "web (copy 2)"
    );

    // End to end: dest already owns "web".
    let src = LauncherStore::open_in_memory().unwrap();
    let mut dest = LauncherStore::open_in_memory().unwrap();
    mk_host(&src, "web", None, None);
    mk_host(&dest, "web", None, None);

    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &host_selection(&["web"]),
        TransferMode::Copy,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].dest_name, "web (copy 1)");
    assert!(plan.items[0].renamed);

    let mut src = src;
    let result = apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap();
    assert_eq!(result.renamed, vec!["web (copy 1)".to_string()]);
    // Both rows survive: the pre-existing dest host plus the renamed copy.
    assert!(dest.get_host_by_name("web").unwrap().is_some());
    assert!(dest.get_host_by_name("web (copy 1)").unwrap().is_some());
    // The renamed row is a full copy (address carried), and the pre-existing
    // dest row is untouched. NOTE: no shared reference oracle exists —
    // production `unique_host_name` uses the `base-2` scheme while transfer
    // renames use `base (copy N)` — so the hand-written vectors above ARE the
    // oracle for the transfer scheme.
    assert_eq!(
        dest.get_host_by_name("web (copy 1)")
            .unwrap()
            .expect("renamed copy")
            .address,
        "web.example.com"
    );
}

#[test]
fn transfer_active_block() {
    let src = LauncherStore::open_in_memory().unwrap();
    let mut dest = LauncherStore::open_in_memory().unwrap();
    let id = mk_identity(&src, "id-a");
    mk_host(&src, "web", Some(id), None);
    mk_host(&src, "db", Some(id), None);

    let blocked_hosts: HashSet<String> = ["web".into()].into_iter().collect();
    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &host_selection(&["web", "db"]),
        TransferMode::Copy,
        GroupTransferMode::GroupOnly,
        &blocked_hosts,
    );
    // The blocked host carries a message; everything else stays runnable.
    let blocked = plan.blocked();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].src_name, "web");
    assert!(blocked[0].blocked.as_ref().unwrap().contains("web"));
    assert!(!plan.runnable().is_empty());

    let mut src = src;
    let result = apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap();
    assert_eq!(result.blocked, vec!["web".to_string()]);
    assert!(dest.get_host_by_name("web").unwrap().is_none());
    assert!(dest.get_host_by_name("db").unwrap().is_some());
    // Source keeps the blocked host.
    assert!(src.get_host_by_name("web").unwrap().is_some());
    // Negative oracle, both sides: the blocked row never reaches dest, stays
    // linked in source; the runnable row transfers exactly once, wired up.
    assert_eq!(result.transferred, 2); // runnable host + auto-carried identity
    assert_eq!(
        src.get_host_by_name("web")
            .unwrap()
            .expect("src host")
            .identity_id,
        Some(id)
    );
    let dest_db_id = dest
        .get_identity_by_name("id-a")
        .unwrap()
        .expect("dest identity")
        .id;
    assert_eq!(
        dest.get_host_by_name("db")
            .unwrap()
            .expect("dest host")
            .identity_id,
        Some(dest_db_id)
    );
}

#[test]
fn transfer_group_only_vs_with_hosts() {
    for group_mode in [GroupTransferMode::GroupOnly, GroupTransferMode::WithHosts] {
        let src = LauncherStore::open_in_memory().unwrap();
        let mut dest = LauncherStore::open_in_memory().unwrap();
        let gid = mk_group(&src, "ops");
        mk_host(&src, "web", None, Some(gid));

        let selection = TransferSelection {
            group_names: vec!["ops".into()],
            ..Default::default()
        };
        let plan = build_transfer_plan(
            &snapshot(&src),
            &dest_index(&dest),
            &selection,
            TransferMode::Copy,
            group_mode,
            &HashSet::new(),
        );
        let has_host_item = plan.items.iter().any(|i| i.kind == "host");

        let mut src = src;
        apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap();

        let dest_group = dest
            .list_groups()
            .unwrap()
            .into_iter()
            .find(|g| g.name == "ops")
            .expect("dest group");
        assert!(!dest_group.reserved);
        match group_mode {
            GroupTransferMode::GroupOnly => {
                // Members stay in source (existing promotion behavior); only the
                // group shell moves.
                assert!(!has_host_item);
                assert!(dest.get_host_by_name("web").unwrap().is_none());
                assert!(src.get_host_by_name("web").unwrap().is_some());
                // Group-only must not disturb the source membership either.
                assert!(
                    src.get_host_by_name("web")
                        .unwrap()
                        .expect("src member")
                        .groups
                        .iter()
                        .any(|g| g.name == "ops"),
                    "member keeps its group in source under GroupOnly"
                );
            }
            GroupTransferMode::WithHosts => {
                assert!(has_host_item);
                let h = dest.get_host_by_name("web").unwrap().expect("dest member");
                assert!(h.groups.iter().any(|g| g.name == "ops"));
                // Copy semantics hold in both modes: source keeps the member.
                assert!(src.get_host_by_name("web").unwrap().is_some());
            }
        }
    }
}
fn mk_tunnel_unlabeled(store: &LauncherStore, host_id: i64, local_port: u16) -> i64 {
    store
        .create_tunnel(&NewTunnel {
            host_id: Some(host_id),
            tunnel_type: TunnelType::Local,
            local_port,
            remote_host: "localhost".into(),
            remote_port: 80,
            label: None,
            auto_connect: false,
        })
        .unwrap()
}

fn tunnel_selection(names: &[&str]) -> TransferSelection {
    TransferSelection {
        tunnel_names: names.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn transfer_unlabeled_tunnel_by_id() {
    // Oracle: the source tunnel row — an unlabeled tunnel has no label to
    // select, so the engine must resolve its bare-id token (`tunnel#<id>` is
    // the stored fallback name, but callers pass the bare id).
    let src = LauncherStore::open_in_memory().unwrap();
    let mut dest = LauncherStore::open_in_memory().unwrap();
    mk_host(&src, "web", None, None);
    let host_id = src.get_host_by_name("web").unwrap().unwrap().id;
    let tid = mk_tunnel_unlabeled(&src, host_id, 8080);

    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &tunnel_selection(&[&tid.to_string()]),
        TransferMode::Copy,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    assert!(
        plan.items.iter().any(|i| i.kind == "tunnel"),
        "bare-id token must scope the unlabeled tunnel, got: {:?}",
        plan.items
    );

    let mut src = src;
    let result = apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap();
    assert!(result.blocked.is_empty());
    let tunnels = dest.list_tunnels().unwrap();
    assert_eq!(tunnels.len(), 1);
    assert_eq!(tunnels[0].local_port, 8080);
    assert_eq!(tunnels[0].label, None);
}

#[test]
fn transfer_unlabeled_tunnel_by_port_token() {
    // Oracle: the source tunnel row — the TUI names an unlabeled tunnel
    // `:<port>`, so the engine must resolve that token back to the row.
    let src = LauncherStore::open_in_memory().unwrap();
    let mut dest = LauncherStore::open_in_memory().unwrap();
    mk_host(&src, "web", None, None);
    let host_id = src.get_host_by_name("web").unwrap().unwrap().id;
    mk_tunnel_unlabeled(&src, host_id, 8080);

    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &tunnel_selection(&[":8080"]),
        TransferMode::Copy,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    assert!(
        plan.items.iter().any(|i| i.kind == "tunnel"),
        "`:<port>` token must scope the unlabeled tunnel, got: {:?}",
        plan.items
    );

    let mut src = src;
    let result = apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap();
    assert!(result.blocked.is_empty());
    assert_eq!(dest.list_tunnels().unwrap().len(), 1);
}

fn file_store(subdir: &str) -> (tempfile::TempDir, LauncherStore, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join(subdir).join("launcher.db");
    let store = LauncherStore::open(&db_path).unwrap();
    (dir, store, db_path)
}

fn fail_identity_inserts(db_path: &std::path::Path) {
    // Oracle: real SQLite — a BEFORE INSERT trigger fails every identity
    // insert inside the engine's own destination transaction, exercising the
    // rollback path no mock can reach.
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute(
        "CREATE TRIGGER transfer_test_fail_identity BEFORE INSERT ON identities \
         BEGIN SELECT RAISE(ABORT, 'transfer test: identity insert refused'); END",
        [],
    )
    .unwrap();
}

fn dest_identity_names(dest: &LauncherStore) -> Vec<String> {
    dest.list_identities()
        .unwrap()
        .into_iter()
        .map(|i| i.name)
        .collect()
}

#[test]
fn transfer_unknown_selection_plans_nothing() {
    // Oracle: the empty plan — unknown names are ignored, so the confirm
    // layers (TUI `y`, CLI) must refuse instead of applying zero items.
    let src = LauncherStore::open_in_memory().unwrap();
    let dest = LauncherStore::open_in_memory().unwrap();
    mk_host(&src, "web", None, None);
    let selection = TransferSelection {
        host_names: vec!["ghost".into()],
        group_names: vec!["ghost-group".into()],
        identity_names: vec!["ghost-id".into()],
        tunnel_names: vec![":9999".into(), "tunnel#424242".into()],
    };
    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &selection,
        TransferMode::Copy,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    assert!(
        plan.items.is_empty(),
        "unknown names plan nothing: {:?}",
        plan.items
    );
    assert!(plan.runnable().is_empty());
}

#[test]
fn transfer_failing_identity_blocks_host() {
    // Oracle: real SQLite rows — with identity inserts refused, the host must
    // NOT land wired to `None`; nothing may survive in dest.
    let src = LauncherStore::open_in_memory().unwrap();
    let (_dest_dir, mut dest, dest_db) = file_store("dest");
    let id = mk_identity(&src, "id-a");
    mk_host(&src, "web", Some(id), None);
    fail_identity_inserts(&dest_db);

    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &host_selection(&["web"]),
        TransferMode::Copy,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    assert_eq!(plan.runnable().len(), 2); // host + auto-carried identity

    let mut src = src;
    let error = apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("id-a") && message.contains("rolled back"),
        "error names the failed item and the rollback: {message}"
    );
    // No partial rows: neither the identity nor its host survived.
    assert!(!dest_identity_names(&dest).contains(&"id-a".to_string()));
    assert!(dest.get_host_by_name("web").unwrap().is_none());
    // Source untouched (copy keeps originals regardless).
    assert!(src.get_host_by_name("web").unwrap().is_some());
    assert!(src.get_identity_by_name("id-a").unwrap().is_some());
}

#[test]
fn transfer_failure_rolls_back_clean_items() {
    // Oracle: real SQLite rows — the group insert succeeds on its own, but
    // the failing identity must drag it back out via rollback.
    let src = LauncherStore::open_in_memory().unwrap();
    let (_dest_dir, mut dest, dest_db) = file_store("dest");
    let gid = mk_group(&src, "ops");
    let id = mk_identity(&src, "id-a");
    mk_host(&src, "web", Some(id), Some(gid));
    fail_identity_inserts(&dest_db);

    let selection = TransferSelection {
        group_names: vec!["ops".into()],
        ..Default::default()
    };
    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &selection,
        TransferMode::Copy,
        GroupTransferMode::WithHosts,
        &HashSet::new(),
    );
    assert!(plan.items.iter().any(|i| i.kind == "group"));

    let mut src = src;
    apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap_err();
    assert!(
        dest.list_groups()
            .unwrap()
            .into_iter()
            .all(|g| g.name != "ops"),
        "clean group insert rolled back with the failing identity"
    );
    assert!(dest.get_host_by_name("web").unwrap().is_none());
    assert!(!dest_identity_names(&dest).contains(&"id-a".to_string()));
}

#[test]
fn transfer_move_delete_skipped_on_failure() {
    // Oracle: real SQLite rows on both sides — a failed destination build
    // must leave the source fully intact (no move-deletion at all).
    let src = LauncherStore::open_in_memory().unwrap();
    let (_dest_dir, mut dest, dest_db) = file_store("dest");
    let id = mk_identity(&src, "id-a");
    mk_host(&src, "web", Some(id), None);
    fail_identity_inserts(&dest_db);

    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &host_selection(&["web"]),
        TransferMode::Move,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );

    let mut src = src;
    apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Move).unwrap_err();
    assert!(src.get_host_by_name("web").unwrap().is_some());
    assert!(src.get_identity_by_name("id-a").unwrap().is_some());
    assert!(dest.get_host_by_name("web").unwrap().is_none());
}

#[test]
fn transfer_blocked_host_identity_not_carried() {
    // Oracle: the plan items — a blocked host stays home, so its identity
    // must not ride along without it (unless a runnable host needs it).
    let src = LauncherStore::open_in_memory().unwrap();
    let mut dest = LauncherStore::open_in_memory().unwrap();
    let solo = mk_identity(&src, "solo");
    let shared = mk_identity(&src, "shared");
    mk_host(&src, "web", Some(solo), None);
    mk_host(&src, "db", Some(shared), None);

    let blocked_hosts: HashSet<String> = ["web".into()].into_iter().collect();
    let plan = build_transfer_plan(
        &snapshot(&src),
        &dest_index(&dest),
        &host_selection(&["web", "db"]),
        TransferMode::Copy,
        GroupTransferMode::GroupOnly,
        &blocked_hosts,
    );
    let identity_items: Vec<&str> = plan
        .items
        .iter()
        .filter(|i| i.kind == "identity")
        .map(|i| i.src_name.as_str())
        .collect();
    assert_eq!(
        identity_items,
        vec!["shared"],
        "only the runnable host's identity is carried: {identity_items:?}"
    );

    let mut src = src;
    let result = apply_transfer_plan(&mut src, &mut dest, &plan, TransferMode::Copy).unwrap();
    assert_eq!(result.blocked, vec!["web".to_string()]);
    assert!(dest.get_identity_by_name("shared").unwrap().is_some());
    assert!(dest.get_identity_by_name("solo").unwrap().is_none());
    assert!(dest.get_host_by_name("db").unwrap().is_some());
}
