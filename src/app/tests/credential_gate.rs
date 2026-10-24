//! The unlock gate on every TUI path that reads a stored secret (RECONCILE
//! 3.D): a synced credential that is still locked blocks the path; once it is
//! open the path goes on by itself; a local secret is never in the way.

use super::*;
use crate::app::sync::SyncIntent;
use crate::credentials::{host_key, PasswordStore, QuorumGatedStore, SecretGate, SecretRef};
use crate::store::{ManagedHost, NewHost};
use zeroize::Zeroizing;

/// An app whose store is a gate over an in-memory store; nothing is synced
/// until `guard` says so.
fn gated_app() -> (App, Arc<SecretGate>, Arc<MemoryPasswordStore>) {
    let secrets = Arc::new(MemoryPasswordStore::default());
    let gated = QuorumGatedStore::new(Box::new(SharedStore(secrets.clone())));
    let gate = gated.gate_handle();
    let app = test_app_with_store(vec![], Box::new(gated));
    (app, gate, secrets)
}

fn add_host(app: &mut App, name: &str, jump: Option<&str>) -> ManagedHost {
    let mut h = NewHost::launcher(name, "127.0.0.1");
    h.port = Some(1); // nothing listens: a spawned ssh just fails
    h.username = Some("ops".into());
    h.proxy_jump = jump.map(String::from);
    let managed = app.store().create_host(&h).unwrap();
    app.reload_hosts().unwrap();
    managed
}

/// Treat exactly the given hosts' secrets as synced credentials.
fn guard(gate: &SecretGate, hosts: &[&ManagedHost]) {
    let keys: Vec<String> = hosts.iter().map(|h| host_key(h.id)).collect();
    gate.set_synced_check(move |k| keys.iter().any(|g| g == k));
}

fn unlock(gate: &SecretGate, host: &ManagedHost, pw: &str) {
    gate.insert(host_key(host.id), Zeroizing::new(pw.to_string()));
}

fn says_second_share(app: &App) -> bool {
    app.host_notice
        .as_deref()
        .is_some_and(|n| n.contains("needs a second share"))
}

/// Run the blocked path with no worker (it refuses and says why), then with a
/// worker (modal + queued request), and return with the modal still open.
fn block_twice(app: &mut App, key: &str, mut path: impl FnMut(&mut App)) {
    path(app);
    assert!(says_second_share(app), "{:?}", app.host_notice);
    assert_ne!(app.mode, AppMode::SyncUnlock, "no worker, no modal");
    app.sync.worker = true;
    app.host_notice = None;
    path(app);
    assert_eq!(app.mode, AppMode::SyncUnlock);
    assert_eq!(
        app.sync.intents.last(),
        Some(&SyncIntent::Unlock {
            secrets: vec![key.to_string()]
        })
    );
}

#[test]
fn connect_waits_for_a_locked_credential_and_goes_on_once_it_is_open() {
    let (mut app, gate, _) = gated_app();
    let web = add_host(&mut app, "web", None);
    guard(&gate, &[&web]);
    let entry = HostEntry::from_managed(web.clone());
    block_twice(&mut app, &host_key(web.id), |a| {
        a.connect_host_entry(entry.clone()).unwrap();
    });
    assert!(app.sessions.is_empty(), "nothing was spawned while locked");
    unlock(&gate, &web, "pw");
    app.poll_sync();
    assert_eq!(app.sessions.len(), 1, "the connect resumed by itself");
    assert_ne!(app.mode, AppMode::SyncUnlock);
}

#[test]
fn a_local_secret_is_never_in_the_way() {
    let (mut app, gate, secrets) = gated_app();
    let local = add_host(&mut app, "local", None);
    let synced = add_host(&mut app, "synced", None);
    secrets.set(&host_key(local.id), "local-pw").unwrap();
    guard(&gate, &[&synced]);
    app.connect_host_entry(HostEntry::from_managed(local))
        .unwrap();
    assert_eq!(app.sessions.len(), 1, "connected without any unlock");
    assert!(app.unlock_resume.is_none() && app.sync.intents.is_empty());
}

#[test]
fn a_jump_hops_locked_credential_blocks_the_connect_too() {
    let (mut app, gate, _) = gated_app();
    let lb = add_host(&mut app, "lb-1", None);
    let target = add_host(&mut app, "db-1", Some("lb-1"));
    guard(&gate, &[&lb]); // only the hop is synced
    let entry = HostEntry::from_managed(target);
    block_twice(&mut app, &host_key(lb.id), |a| {
        a.connect_host_entry(entry.clone()).unwrap();
    });
    unlock(&gate, &lb, "lb-pw");
    app.poll_sync();
    assert_eq!(app.sessions.len(), 1);
}

#[test]
fn sftp_waits_for_a_locked_credential() {
    let (mut app, gate, _) = gated_app();
    let web = add_host(&mut app, "web", None);
    guard(&gate, &[&web]);
    app.active_tab = 1;
    block_twice(&mut app, &host_key(web.id), |a| {
        if a.mode != AppMode::SyncUnlock {
            a.handle_key(key(KeyCode::Enter)).unwrap();
        }
    });
    assert!(app.sftp.is_none(), "no browser while locked");
    unlock(&gate, &web, "pw");
    app.poll_sync();
    assert!(app.sftp.is_some(), "the connect resumed by itself");
}

#[test]
fn the_second_sftp_pane_waits_for_a_locked_credential() {
    let (mut app, gate, _) = gated_app();
    let web = add_host(&mut app, "web", None);
    guard(&gate, &[&web]);
    app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));
    let idx = app.hosts.iter().position(|h| h.name() == "web").unwrap();
    block_twice(&mut app, &host_key(web.id), |a| {
        a.sftp_connect_left_pane(idx).unwrap();
    });
    assert!(app.sftp_tx2.is_none());
    unlock(&gate, &web, "pw");
    app.poll_sync();
    assert!(app.sftp_tx2.is_some());
}

#[test]
fn broadcast_waits_for_locked_credentials_and_then_carries_the_secret() {
    let (mut app, gate, _) = gated_app();
    let web = add_host(&mut app, "web", None);
    guard(&gate, &[&web]);
    block_twice(&mut app, &host_key(web.id), |a| {
        if a.mode != AppMode::SyncUnlock {
            a.open_broadcast_for_hosts(vec![web.id], "web".into());
        }
    });
    unlock(&gate, &web, "pw");
    app.poll_sync();
    assert_eq!(app.mode, AppMode::BroadcastCommand);
    let setup = app.broadcast_setup.as_ref().unwrap();
    assert_eq!(setup.candidates.len(), 1);
    assert_eq!(
        setup.candidates[0].secret.as_ref().map(|s| s.value()),
        Some("pw"),
        "the unlocked secret reaches the run"
    );
}

#[test]
fn push_key_waits_for_a_locked_credential() {
    let (mut app, gate, _) = gated_app();
    let web = add_host(&mut app, "web", None);
    guard(&gate, &[&web]);
    let entry = HostEntry::from_managed(web.clone());
    // No such key file: past the gate the push fails reading it, which is how
    // the test sees that it went on.
    let identity = crate::store::Identity {
        id: 1,
        name: "k".into(),
        username: None,
        private_key: Some(std::path::PathBuf::from("/nonexistent/sshub-test-key")),
        certificate: None,
        has_password: false,
    };
    block_twice(&mut app, &host_key(web.id), |a| {
        a.push_public_key_to_host(&entry, &identity).unwrap();
    });
    assert!(!app
        .host_notice
        .as_deref()
        .unwrap_or("")
        .contains("Failed to read public key"));
    unlock(&gate, &web, "pw");
    app.poll_sync();
    assert!(
        app.host_notice
            .as_deref()
            .is_some_and(|n| n.contains("Failed to read public key")),
        "{:?}",
        app.host_notice
    );
}

fn add_tunnel(app: &mut App, host: &ManagedHost, port: u16, auto: bool) -> i64 {
    let id = app
        .store()
        .create_tunnel(&crate::store::NewTunnel {
            host_id: Some(host.id),
            tunnel_type: crate::store::TunnelType::Local,
            local_port: port,
            remote_host: "localhost".into(),
            remote_port: 80,
            label: None,
            auto_connect: auto,
        })
        .unwrap();
    app.reload_tunnels().unwrap();
    id
}

#[test]
fn a_tunnel_start_waits_for_a_locked_credential() {
    let (mut app, gate, _) = gated_app();
    let web = add_host(&mut app, "web", None);
    guard(&gate, &[&web]);
    let id = add_tunnel(&mut app, &web, 18991, false);
    block_twice(&mut app, &host_key(web.id), |a| {
        a.toggle_tunnel_by_id(id).unwrap();
    });
    assert!(!app.tunnel_manager.has_child(id) && !app.tunnel_manager.is_running(id));
    assert!(app.tunnel_notice.is_none(), "no start was attempted");
    unlock(&gate, &web, "pw");
    app.poll_sync();
    assert!(
        app.tunnel_notice.is_some(),
        "the start resumed by itself and reported"
    );
}

#[test]
fn auto_started_tunnels_hold_while_locked_and_start_after() {
    let (mut app, gate, _) = gated_app();
    let web = add_host(&mut app, "web", None);
    guard(&gate, &[&web]);
    let id = add_tunnel(&mut app, &web, 18992, true);
    assert!(app.tunnel_credentials_locked(&app.tunnels[0].clone()));
    app.bootstrap_auto_connect_tunnels().unwrap();
    assert!(!app.tunnel_manager.has_child(id) && !app.tunnel_manager.is_reconnecting(id));
    assert!(
        app.tunnel_notice
            .as_deref()
            .is_some_and(|n| n.contains("waits for its credentials")),
        "{:?}",
        app.tunnel_notice
    );
    unlock(&gate, &web, "pw");
    assert!(!app.tunnel_credentials_locked(&app.tunnels[0].clone()));
    app.bootstrap_auto_connect_tunnels().unwrap();
    assert!(
        app.tunnel_manager.has_child(id)
            || app.tunnel_manager.is_running(id)
            || app.tunnel_manager.is_reconnecting(id)
    );
}

#[test]
fn the_tunnel_check_costs_nothing_while_nothing_is_synced() {
    let (mut app, _gate, _) = gated_app();
    let web = add_host(&mut app, "web", None);
    add_tunnel(&mut app, &web, 18993, false);
    assert!(!app.tunnel_credentials_locked(&app.tunnels[0].clone()));
}

#[test]
fn an_unlocked_hop_secret_is_staged_for_the_helper_and_a_locked_one_is_not() {
    let (mut app, gate, _) = gated_app();
    let lb1 = add_host(&mut app, "lb-1", None);
    let lb2 = add_host(&mut app, "lb-2", None);
    let target = add_host(&mut app, "db-1", Some("lb-1,lb-2"));
    guard(&gate, &[&lb1, &lb2]);
    unlock(&gate, &lb1, "lb-1-pw"); // lb-2 stays locked
    let jump = app
        .store()
        .resolve_connection(&target)
        .unwrap()
        .proxy_jump
        .clone();
    let hops = hop_secrets(
        app.store(),
        &app.hosts,
        app.password_store.as_ref(),
        jump.as_deref(),
    );
    assert_eq!(hops.len(), 1, "the locked hop is left to the user");
    assert!(matches!(hops[0].refs.as_slice(), [SecretRef::Staged(_)]));
    assert!(
        !format!("{hops:?}").contains("lb-1-pw"),
        "Debug never prints it"
    );

    // It travels to the helper the way the destination's own secret does.
    let staged = crate::session::askpass::AskpassSecret::staged(None, &hops).unwrap();
    let text = std::fs::read_to_string(staged.path()).unwrap();
    assert!(text.contains("v:lb-1-pw"), "{text}");
    assert!(
        !text.contains("host:"),
        "no store reference for the staged hop"
    );
}

#[test]
fn cancelling_the_unlock_drops_the_waiting_action() {
    let (mut app, gate, _) = gated_app();
    let web = add_host(&mut app, "web", None);
    guard(&gate, &[&web]);
    app.sync.worker = true;
    app.connect_host_entry(HostEntry::from_managed(web.clone()))
        .unwrap();
    assert_eq!(app.mode, AppMode::SyncUnlock);
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.unlock_resume.is_none());
    unlock(&gate, &web, "pw");
    app.poll_sync();
    assert!(app.sessions.is_empty(), "a cancelled connect never resumes");
}
