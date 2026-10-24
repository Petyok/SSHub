//! A move must clear the moved hosts'/identities' saved secrets from the
//! SOURCE credential store; a copy must delete nothing; failures are reported.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use anyhow::Result;
use sshub::credentials::{host_key, identity_key, PasswordStore};
use sshub::store::transfer::{
    apply_transfer_plan, apply_transfer_plan_with_secrets, build_transfer_plan,
    secret_purge_warning, DestIndex, GroupTransferMode, SecretStores, TransferMode,
    TransferSelection, TransferSnapshot,
};
use sshub::store::{LauncherStore, NewHost, NewIdentity};

/// In-memory credential store; keys in `fail` refuse deletion.
#[derive(Default)]
struct FakeSecrets {
    map: Mutex<HashMap<String, String>>,
    fail: HashSet<String>,
    /// Refuses every write (a locked destination keyring).
    refuse_set: bool,
}

impl PasswordStore for FakeSecrets {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.map.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, password: &str) -> Result<()> {
        if self.refuse_set {
            anyhow::bail!("keyring locked");
        }
        self.map.lock().unwrap().insert(key.into(), password.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        if self.fail.contains(key) {
            anyhow::bail!("keyring locked");
        }
        self.map.lock().unwrap().remove(key);
        Ok(())
    }
}

fn run(
    src: &mut LauncherStore,
    mode: TransferMode,
    hosts: &[&str],
) -> sshub::store::TransferResult {
    let mut dest = LauncherStore::open_in_memory().unwrap();
    let plan = build_transfer_plan(
        &TransferSnapshot::capture(src).unwrap(),
        &DestIndex::capture(&dest, "dest").unwrap(),
        &TransferSelection {
            host_names: hosts.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        },
        mode,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    apply_transfer_plan(src, &mut dest, &plan, mode).unwrap()
}

fn host(store: &LauncherStore, name: &str, identity_id: Option<i64>) -> i64 {
    store
        .create_host(&NewHost {
            identity_id,
            address: format!("{name}.example.com"),
            ..NewHost::launcher(name, "")
        })
        .unwrap()
        .id
}

#[test]
fn move_purges_host_and_orphaned_identity_secrets_but_keeps_the_rest() {
    let mut src = LauncherStore::open_in_memory().unwrap();
    let ident = src
        .create_identity(&NewIdentity {
            name: "solo".into(),
            username: None,
            private_key: None,
            certificate: None,
            sort_order: 0,
            has_password: true,
        })
        .unwrap()
        .id;
    let moved = host(&src, "moved", Some(ident));
    let stay = host(&src, "stay", None);
    let secrets = FakeSecrets::default();
    for k in [host_key(moved), host_key(stay), identity_key(ident)] {
        secrets.set(&k, "pw").unwrap();
    }

    let result = run(&mut src, TransferMode::Move, &["moved"]);
    assert!(result.purge_source_secrets(&secrets).is_empty());

    assert_eq!(secrets.get(&host_key(moved)).unwrap(), None);
    assert_eq!(secrets.get(&identity_key(ident)).unwrap(), None);
    assert_eq!(secrets.get(&host_key(stay)).unwrap().as_deref(), Some("pw"));
}

#[test]
fn move_keeps_secret_of_identity_still_used_by_source_host() {
    let mut src = LauncherStore::open_in_memory().unwrap();
    let ident = src
        .create_identity(&NewIdentity {
            name: "shared".into(),
            username: None,
            private_key: None,
            certificate: None,
            sort_order: 0,
            has_password: true,
        })
        .unwrap()
        .id;
    host(&src, "moved", Some(ident));
    host(&src, "stay", Some(ident));
    let secrets = FakeSecrets::default();
    secrets.set(&identity_key(ident), "pw").unwrap();

    let result = run(&mut src, TransferMode::Move, &["moved"]);
    assert!(result.purge_source_secrets(&secrets).is_empty());
    assert!(secrets.get(&identity_key(ident)).unwrap().is_some());
}

#[test]
fn copy_deletes_no_secrets() {
    let mut src = LauncherStore::open_in_memory().unwrap();
    let id = host(&src, "web", None);
    let secrets = FakeSecrets::default();
    secrets.set(&host_key(id), "pw").unwrap();

    let result = run(&mut src, TransferMode::Copy, &["web"]);
    assert!(result.deleted_host_ids.is_empty());
    assert!(result.purge_source_secrets(&secrets).is_empty());
    assert!(secrets.get(&host_key(id)).unwrap().is_some());
}

#[test]
fn purge_failures_are_reported_not_swallowed() {
    let mut src = LauncherStore::open_in_memory().unwrap();
    let a = host(&src, "a", None);
    let b = host(&src, "b", None);
    let mut secrets = FakeSecrets::default();
    secrets.fail.insert(host_key(a));
    secrets.set(&host_key(a), "pw").unwrap();
    secrets.set(&host_key(b), "pw").unwrap();

    let result = run(&mut src, TransferMode::Move, &["a", "b"]);
    let failures = result.purge_source_secrets(&secrets);

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains(&host_key(a)));
    // The healthy key is still purged despite the other failing.
    assert_eq!(secrets.get(&host_key(b)).unwrap(), None);
    assert!(secret_purge_warning(&failures).starts_with("1 moved secret(s)"));
}

/// Move one host (and its identity) into a fresh destination whose ids are
/// padded away from the source's, keeping the destination store for read-back.
fn move_into_fresh_dest(
    src: &mut LauncherStore,
    host_name: &str,
    secrets: Option<(&FakeSecrets, &FakeSecrets)>,
) -> (sshub::store::TransferResult, LauncherStore) {
    let mut dest = LauncherStore::open_in_memory().unwrap();
    // A carry that wrote under the source id would otherwise still pass.
    for pad in ["pad-a", "pad-b", "pad-c"] {
        host(&dest, pad, None);
    }
    dest.create_identity(&NewIdentity {
        name: "pad-identity".into(),
        username: None,
        private_key: None,
        certificate: None,
        sort_order: 0,
        has_password: false,
    })
    .unwrap();
    let plan = build_transfer_plan(
        &TransferSnapshot::capture(src).unwrap(),
        &DestIndex::capture(&dest, "dest").unwrap(),
        &TransferSelection {
            host_names: vec![host_name.into()],
            ..Default::default()
        },
        TransferMode::Move,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    let result = apply_transfer_plan_with_secrets(
        src,
        &mut dest,
        &plan,
        TransferMode::Move,
        secrets.map(|(src, dest)| SecretStores { src, dest }),
    )
    .unwrap();
    (result, dest)
}

fn secret_identity(store: &LauncherStore) -> i64 {
    store
        .create_identity(&NewIdentity {
            name: "deploy".into(),
            username: None,
            private_key: None,
            certificate: None,
            sort_order: 0,
            has_password: true,
        })
        .unwrap()
        .id
}

/// A host whose row flags a saved password.
fn secret_host(store: &LauncherStore, name: &str, identity_id: Option<i64>) -> i64 {
    store
        .create_host(&NewHost {
            identity_id,
            has_password: true,
            address: format!("{name}.example.com"),
            ..NewHost::launcher(name, "")
        })
        .unwrap()
        .id
}

#[test]
fn move_carries_host_and_identity_secrets_to_the_destination() {
    let mut src = LauncherStore::open_in_memory().unwrap();
    let ident = secret_identity(&src);
    let web = secret_host(&src, "web", Some(ident));
    let src_secrets = FakeSecrets::default();
    src_secrets.set(&host_key(web), "host-pw").unwrap();
    src_secrets.set(&identity_key(ident), "key-pass").unwrap();
    let dest_secrets = FakeSecrets::default();

    let (result, dest) = move_into_fresh_dest(&mut src, "web", Some((&src_secrets, &dest_secrets)));
    assert!(result.carry_failures.is_empty() && result.kept_in_source.is_empty());
    assert!(result.purge_source_secrets(&src_secrets).is_empty());

    let dest_host = dest.get_host_by_name("web").unwrap().unwrap();
    let dest_ident = dest.get_identity_by_name("deploy").unwrap().unwrap();
    assert_ne!(
        dest_host.id, web,
        "ids must differ for this test to mean anything"
    );
    assert_ne!(dest_ident.id, ident);
    assert!(dest_host.has_password && dest_ident.has_password);
    assert_eq!(
        dest_secrets
            .get(&host_key(dest_host.id))
            .unwrap()
            .as_deref(),
        Some("host-pw")
    );
    assert_eq!(
        dest_secrets
            .get(&identity_key(dest_ident.id))
            .unwrap()
            .as_deref(),
        Some("key-pass")
    );
    // The source is cleared only after the carry.
    assert!(src.get_host_by_name("web").unwrap().is_none());
    assert_eq!(src_secrets.get(&host_key(web)).unwrap(), None);
    assert_eq!(src_secrets.get(&identity_key(ident)).unwrap(), None);
}

#[test]
fn a_remembered_secret_on_an_unflagged_row_is_still_carried() {
    // "Remember" saves the secret without setting the row flag; presence
    // decides. Skipping unflagged rows would purge the only copy.
    let mut src = LauncherStore::open_in_memory().unwrap();
    let web = host(&src, "web", None); // has_password = false
    let src_secrets = FakeSecrets::default();
    src_secrets.set(&host_key(web), "remembered").unwrap();
    let dest_secrets = FakeSecrets::default();

    let (result, dest) = move_into_fresh_dest(&mut src, "web", Some((&src_secrets, &dest_secrets)));
    assert!(result.carry_failures.is_empty());
    assert!(result.purge_source_secrets(&src_secrets).is_empty());
    let dest_host = dest.get_host_by_name("web").unwrap().unwrap();
    assert!(dest_host.has_password, "the destination row now says so");
    assert_eq!(
        dest_secrets
            .get(&host_key(dest_host.id))
            .unwrap()
            .as_deref(),
        Some("remembered")
    );
    assert_eq!(
        src_secrets.get(&host_key(web)).unwrap(),
        None,
        "purged only after"
    );
}

#[test]
fn rows_with_no_secret_anywhere_move_cleanly() {
    let mut src = LauncherStore::open_in_memory().unwrap();
    host(&src, "web", None);
    let dest_secrets = FakeSecrets::default();
    let (result, dest) = move_into_fresh_dest(
        &mut src,
        "web",
        Some((&FakeSecrets::default(), &dest_secrets)),
    );
    assert!(result.carry_failures.is_empty() && result.kept_in_source.is_empty());
    assert!(!dest.get_host_by_name("web").unwrap().unwrap().has_password);
    assert!(dest_secrets.map.lock().unwrap().is_empty());
    assert!(src.get_host_by_name("web").unwrap().is_none());
}

#[test]
fn an_unreadable_source_store_keeps_the_host_instead_of_purging() {
    struct Unreadable;
    impl PasswordStore for Unreadable {
        fn get(&self, _: &str) -> Result<Option<String>> {
            anyhow::bail!("keyring locked")
        }
        fn set(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    let mut src = LauncherStore::open_in_memory().unwrap();
    host(&src, "web", None);
    let dest_secrets = FakeSecrets::default();
    let mut dest = LauncherStore::open_in_memory().unwrap();
    let plan = build_transfer_plan(
        &TransferSnapshot::capture(&src).unwrap(),
        &DestIndex::capture(&dest, "dest").unwrap(),
        &TransferSelection {
            host_names: vec!["web".into()],
            ..Default::default()
        },
        TransferMode::Move,
        GroupTransferMode::GroupOnly,
        &HashSet::new(),
    );
    let result = apply_transfer_plan_with_secrets(
        &mut src,
        &mut dest,
        &plan,
        TransferMode::Move,
        Some(SecretStores {
            src: &Unreadable,
            dest: &dest_secrets,
        }),
    )
    .unwrap();
    assert_eq!(result.kept_in_source, ["web"]);
    assert!(src.get_host_by_name("web").unwrap().is_some());
    assert!(dest.get_host_by_name("web").unwrap().is_none());
}

#[test]
fn a_secret_that_cannot_be_carried_keeps_its_source_rows_and_secrets() {
    let mut src = LauncherStore::open_in_memory().unwrap();
    let ident = secret_identity(&src);
    let web = secret_host(&src, "web", Some(ident));
    src.create_tunnel(&sshub::store::NewTunnel {
        host_id: Some(web),
        tunnel_type: sshub::store::TunnelType::Local,
        local_port: 8080,
        remote_host: "localhost".into(),
        remote_port: 80,
        label: Some("pg".into()),
        auto_connect: false,
    })
    .unwrap();
    let src_secrets = FakeSecrets::default();
    src_secrets.set(&host_key(web), "host-pw").unwrap();
    src_secrets.set(&identity_key(ident), "key-pass").unwrap();
    let locked = FakeSecrets {
        refuse_set: true,
        ..Default::default()
    };

    let (result, dest) = move_into_fresh_dest(&mut src, "web", Some((&src_secrets, &locked)));
    assert_eq!(
        result.carry_failures.len(),
        2,
        "{:?}",
        result.carry_failures
    );
    assert!(result
        .carry_failures
        .iter()
        .all(|f| !f.contains("host-pw") && !f.contains("key-pass")));
    let mut kept = result.kept_in_source.clone();
    kept.sort();
    assert_eq!(kept, ["deploy", "web"]);
    assert!(result.deleted_host_ids.is_empty() && result.deleted_identity_ids.is_empty());
    assert!(result.purge_source_secrets(&src_secrets).is_empty());

    // Nothing was lost and the move can be repeated: the source host, its
    // tunnel, the identity and both secrets are all still there.
    assert!(src.get_host_by_name("web").unwrap().is_some());
    assert!(src.get_identity_by_name("deploy").unwrap().is_some());
    assert_eq!(src.list_tunnels().unwrap().len(), 1);
    assert_eq!(src.list_tunnels().unwrap()[0].host_id, Some(web));
    assert_eq!(
        src_secrets.get(&host_key(web)).unwrap().as_deref(),
        Some("host-pw")
    );
    assert_eq!(
        src_secrets.get(&identity_key(ident)).unwrap().as_deref(),
        Some("key-pass")
    );
    // The host is not half-moved: no destination copy that a repeated move
    // would duplicate as `web (copy 1)`; the identity copy stays (hosts that
    // did move would point at it).
    assert!(dest.get_host_by_name("web").unwrap().is_none());
    assert!(dest.list_tunnels().unwrap().is_empty());
    // Nothing is left behind that a repeated move would duplicate: the
    // identity copy nothing uses any more is gone too.
    assert!(dest.get_identity_by_name("deploy").unwrap().is_none());
    assert_eq!(result.transferred, 0, "nothing counts as moved");
}

#[test]
fn a_host_follows_its_identity_when_the_identity_cannot_be_carried() {
    // The identity's passphrase is unreadable, the host's own password is fine:
    // moved alone, the host would point at a destination identity without its
    // passphrase. It stays too.
    let mut src = LauncherStore::open_in_memory().unwrap();
    let ident = secret_identity(&src);
    let web = secret_host(&src, "web", Some(ident));
    let src_secrets = FakeSecrets::default();
    src_secrets.set(&host_key(web), "host-pw").unwrap();
    // identity flagged, but no passphrase in the store: a failure for it only
    let dest_secrets = FakeSecrets::default();
    let (result, dest) = move_into_fresh_dest(&mut src, "web", Some((&src_secrets, &dest_secrets)));
    let mut kept = result.kept_in_source.clone();
    kept.sort();
    assert_eq!(kept, ["deploy", "web"]);
    assert!(src.get_host_by_name("web").unwrap().is_some());
    assert!(dest.get_host_by_name("web").unwrap().is_none());
    assert_eq!(
        src_secrets.get(&host_key(web)).unwrap().as_deref(),
        Some("host-pw")
    );
    assert!(
        dest_secrets.map.lock().unwrap().is_empty(),
        "no stray dest secret"
    );
}

#[test]
fn a_kept_host_keeps_its_group_when_the_group_moves_too() {
    use sshub::store::NewHostGroup;
    let mut src = LauncherStore::open_in_memory().unwrap();
    let group = src
        .create_group(&NewHostGroup {
            name: "prod".into(),
            default_username: Some("deploy".into()),
            ..Default::default()
        })
        .unwrap();
    let web = src
        .create_host(&NewHost {
            group_id: Some(group.id),
            has_password: true,
            ..NewHost::launcher("web", "web.example.com")
        })
        .unwrap()
        .id;
    let src_secrets = FakeSecrets::default();
    src_secrets.set(&host_key(web), "pw").unwrap();
    let locked = FakeSecrets {
        refuse_set: true,
        ..Default::default()
    };
    let mut dest = LauncherStore::open_in_memory().unwrap();
    let plan = build_transfer_plan(
        &TransferSnapshot::capture(&src).unwrap(),
        &DestIndex::capture(&dest, "dest").unwrap(),
        &TransferSelection {
            group_names: vec!["prod".into()],
            ..Default::default()
        },
        TransferMode::Move,
        GroupTransferMode::WithHosts,
        &HashSet::new(),
    );
    let result = apply_transfer_plan_with_secrets(
        &mut src,
        &mut dest,
        &plan,
        TransferMode::Move,
        Some(SecretStores {
            src: &src_secrets,
            dest: &locked,
        }),
    )
    .unwrap();
    assert_eq!(result.kept_in_source, ["web"]);
    let kept = src.get_host_by_name("web").unwrap().expect("kept host");
    assert_eq!(kept.group_id, Some(group.id), "still in its group");
    assert!(src.get_group(group.id).unwrap().is_some());
}

#[test]
fn a_flagged_secret_missing_from_the_source_store_keeps_the_host() {
    // The row says a password is saved, but this store does not hold it (the
    // keyring/credentials.json mismatch): that is a failure, not "nothing to
    // carry", or the source row would be deleted with the password stranded.
    let mut src = LauncherStore::open_in_memory().unwrap();
    secret_host(&src, "web", None);
    let (result, _dest) = move_into_fresh_dest(
        &mut src,
        "web",
        Some((&FakeSecrets::default(), &FakeSecrets::default())),
    );
    assert_eq!(result.kept_in_source, ["web"]);
    assert!(src.get_host_by_name("web").unwrap().is_some());
}

/// Every public path that can keep a host behind (a failed carry) must leave
/// each identity and group it planned in at least one profile: a move that
/// strands nothing in the destination must not also delete the source row.
fn assert_nothing_lost(src: &LauncherStore, dest: &LauncherStore) {
    for name in ["deploy"] {
        let found = |s: &LauncherStore| {
            s.list_identities()
                .unwrap()
                .iter()
                .any(|i| i.name.starts_with(name))
        };
        assert!(found(src) || found(dest), "identity {name} lost");
    }
    let found = |s: &LauncherStore| {
        s.list_groups()
            .unwrap()
            .iter()
            .any(|g| g.name.starts_with("prod"))
    };
    assert!(found(src) || found(dest), "group prod lost");
}

/// `web` (flagged password, absent from the source store, so it is kept) and
/// `db` share group `prod`, whose default identity `deploy` has a secret.
fn kept_host_fixture() -> (LauncherStore, FakeSecrets) {
    use sshub::store::NewHostGroup;
    let src = LauncherStore::open_in_memory().unwrap();
    let ident = secret_identity(&src);
    let group = src
        .create_group(&NewHostGroup {
            name: "prod".into(),
            default_identity_id: Some(ident),
            ..Default::default()
        })
        .unwrap();
    for (name, flagged) in [("web", true), ("db", false)] {
        src.create_host(&NewHost {
            group_id: Some(group.id),
            identity_id: Some(ident),
            has_password: flagged,
            ..NewHost::launcher(name, &format!("{name}.example.com"))
        })
        .unwrap();
    }
    let secrets = FakeSecrets::default();
    secrets.set(&identity_key(ident), "pw").unwrap();
    (src, secrets)
}

fn move_with(
    src: &mut LauncherStore,
    selection: TransferSelection,
    groups: GroupTransferMode,
    src_secrets: &FakeSecrets,
) -> LauncherStore {
    let mut dest = LauncherStore::open_in_memory().unwrap();
    let dest_secrets = FakeSecrets::default();
    let plan = build_transfer_plan(
        &TransferSnapshot::capture(src).unwrap(),
        &DestIndex::capture(&dest, "dest").unwrap(),
        &selection,
        TransferMode::Move,
        groups,
        &HashSet::new(),
    );
    apply_transfer_plan_with_secrets(
        src,
        &mut dest,
        &plan,
        TransferMode::Move,
        Some(SecretStores {
            src: src_secrets,
            dest: &dest_secrets,
        }),
    )
    .unwrap();
    dest
}

#[test]
fn guided_move_with_a_kept_host_strands_no_group_or_identity() {
    let (mut src, secrets) = kept_host_fixture();
    let dest = move_with(
        &mut src,
        TransferSelection {
            host_names: vec!["web".into(), "db".into()],
            group_names: vec!["prod".into()],
            identity_names: vec!["deploy".into()],
            no_auto_carry: true,
            ..Default::default()
        },
        GroupTransferMode::GroupOnly,
        &secrets,
    );
    assert!(src.get_host_by_name("web").unwrap().is_some(), "kept");
    assert!(dest.get_host_by_name("db").unwrap().is_some(), "moved");
    assert_nothing_lost(&src, &dest);
}

#[test]
fn single_host_move_with_a_kept_host_strands_no_group_or_identity() {
    let (mut src, secrets) = kept_host_fixture();
    let dest = move_with(
        &mut src,
        TransferSelection {
            host_names: vec!["web".into()],
            ..Default::default()
        },
        GroupTransferMode::GroupOnly,
        &secrets,
    );
    assert!(src.get_host_by_name("web").unwrap().is_some(), "kept");
    assert_nothing_lost(&src, &dest);
}

#[test]
fn group_move_with_hosts_and_a_kept_host_strands_no_group_or_identity() {
    let (mut src, secrets) = kept_host_fixture();
    let dest = move_with(
        &mut src,
        TransferSelection {
            group_names: vec!["prod".into()],
            ..Default::default()
        },
        GroupTransferMode::WithHosts,
        &secrets,
    );
    assert!(src.get_host_by_name("web").unwrap().is_some(), "kept");
    assert_nothing_lost(&src, &dest);
}

#[test]
fn identity_move_whose_secret_is_missing_keeps_the_identity() {
    let (mut src, _) = kept_host_fixture();
    // `deploy` is flagged but the source store holds nothing for it.
    let dest = move_with(
        &mut src,
        TransferSelection {
            identity_names: vec!["deploy".into()],
            ..Default::default()
        },
        GroupTransferMode::GroupOnly,
        &FakeSecrets::default(),
    );
    assert_nothing_lost(&src, &dest);
    assert!(src
        .list_identities()
        .unwrap()
        .iter()
        .any(|i| i.name == "deploy"));
}
