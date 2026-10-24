//! Vault end to end inside the app: `OnboardingDone` sets the ring up, a
//! synced credential stays locked until the master phrase is typed.

use super::*;
use crate::app::sync::{OnboardingView, SyncIntent};
use crate::credentials::{
    host_key, FilePasswordStore, PasswordStore, QuorumGatedStore, SecretGate, SecretRef,
};
use crate::profile::ProfilePaths;
use crate::sync::client::spawn_sync_thread;
use crate::sync::cred_ops::{cred_put, CredPayload};
use crate::sync::crypto::{Argon2Profile, DeviceIdentity};
use crate::sync::identity_store::load_or_init_identity;
use crate::sync::quorum::{parse_ring_secret, vault_pub};
use crate::sync::runtime::{guard_synced_credentials, round_fn, RoundDeps, RING_SECRET_KEY};
use std::sync::Arc;
use std::time::{Duration, Instant};

const FAST: Argon2Profile = Argon2Profile {
    mem_kib: 64,
    iters: 1,
};
const RING: [u8; 16] = [0xab; 16];

fn file_secrets(p: &ProfilePaths) -> Box<dyn PasswordStore> {
    Box::new(FilePasswordStore::new(p.credentials_file()))
}

fn pump(app: &mut App, what: &str, until: impl Fn(&App) -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !until(app) {
        assert!(
            Instant::now() < end,
            "timed out waiting for {what}; error {:?}, notice {:?}",
            app.sync.last_error,
            app.host_notice
        );
        app.poll_sync();
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Fixture {
    app: App,
    gate: Arc<SecretGate>,
    paths: ProfilePaths,
    phrase: String,
    _dir: tempfile::TempDir,
}

/// An app on a throwaway profile with a gated store and the real worker.
fn fixture() -> Fixture {
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
    let gated = QuorumGatedStore::new(file_secrets(&paths));
    let gate = gated.gate_handle();
    let mut app = test_app_with_store(vec![], Box::new(gated));
    guard_synced_credentials(&gate, app.store.clone());
    app.profile = Some(paths.clone());
    let deps = RoundDeps {
        paths: paths.clone(),
        store: app.store.clone(),
        gate: gate.clone(),
        argon: FAST,
        open_secrets: file_secrets,
        ssh_program: None,
    };
    app.attach_sync_worker(spawn_sync_thread(round_fn(deps)));
    let view = OnboardingView::with_secrets(
        &[0x21; 10],
        RING,
        [0, 1],
        ("/unused/a".into(), "/unused/b".into()),
    )
    .unwrap();
    let phrase = view.secrets().phrase.to_string();
    app.sync
        .intents
        .push(SyncIntent::OnboardingDone(view.secrets().clone()));
    Fixture {
        app,
        gate,
        paths,
        phrase,
        _dir: dir,
    }
}

fn all_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            all_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn onboarding_sets_up_the_vault_ring_and_keeps_the_phrase_off_disk() {
    let mut f = fixture();
    pump(&mut f.app, "the ring", |a| a.sync.configured);
    assert!(vault_pub(&f.app.store).unwrap().is_some());
    let stored = file_secrets(&f.paths)
        .get(RING_SECRET_KEY)
        .unwrap()
        .unwrap();
    assert_eq!(parse_ring_secret(&stored).unwrap(), RING);
    assert_eq!(
        f.app.sync.devices.len(),
        1,
        "this device is the ring's first entry"
    );
    assert!(
        crate::sync::identity_store::key_path(&f.paths).exists(),
        "the device key was created"
    );
    let mut files = Vec::new();
    all_files(f.paths.root.as_path(), &mut files);
    for file in files {
        let bytes = std::fs::read(&file).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(&f.phrase),
            "{} holds the phrase",
            file.display()
        );
    }
}

/// A host whose password is a synced Vault credential, and the gated key.
fn synced_host(f: &Fixture) -> String {
    let host = f
        .app
        .store
        .create_host(&crate::store::NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    let uid = f.app.store.host_uid(host.id).unwrap().unwrap();
    let id: DeviceIdentity =
        load_or_init_identity(&f.paths, file_secrets(&f.paths).as_ref()).unwrap();
    cred_put(
        &f.app.store,
        &id,
        &uid,
        &CredPayload {
            password: Some("s3cret".into()),
            private_key: None,
        },
    )
    .unwrap();
    host_key(host.id)
}

fn type_and_enter(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
}

#[test]
fn a_synced_credential_opens_with_the_master_phrase_and_the_waiting_action_resumes() {
    let mut f = fixture();
    pump(&mut f.app, "the ring", |a| a.sync.configured);
    let key = synced_host(&f);
    assert!(f.gate.get(&key).is_none());

    let blocked = f
        .app
        .preflight_unlock(vec![SecretRef::Key(key.clone())], "web", |app| {
            app.host_notice = Some("resumed".into());
            Ok(())
        });
    assert!(!blocked, "a locked synced credential blocks the action");
    assert_eq!(f.app.mode, AppMode::SyncUnlock);
    pump(&mut f.app, "the prompt", |a| {
        a.sync.unlock.as_ref().is_some_and(|u| !u.paths.is_empty())
    });

    // A wrong phrase fails and the modal stays for another try.
    type_and_enter(&mut f.app, "zebra yankee xray whiskey victor uniform");
    pump(&mut f.app, "the failure", |a| {
        a.host_notice
            .as_deref()
            .is_some_and(|n| n.contains("unlock failed"))
    });
    assert!(f.gate.get(&key).is_none(), "still locked");
    assert_eq!(f.app.mode, AppMode::SyncUnlock);

    // Asking again restarts the unlock; the right phrase opens it.
    let phrase = f.phrase.clone();
    pump(&mut f.app, "the round to end", |a| !a.sync.unlock_running);
    type_and_enter(&mut f.app, &phrase);
    pump(&mut f.app, "the action to resume", |a| {
        a.host_notice.as_deref() == Some("resumed")
    });
    assert_eq!(
        f.gate.get(&key).as_deref().map(String::as_str),
        Some("s3cret")
    );
    assert_eq!(f.app.mode, AppMode::Normal);
}

#[test]
fn closing_the_unlock_modal_abandons_the_round_and_leaves_the_credential_locked() {
    let mut f = fixture();
    pump(&mut f.app, "the ring", |a| a.sync.configured);
    let key = synced_host(&f);
    assert!(!f
        .app
        .preflight_unlock(vec![SecretRef::Key(key.clone())], "web", |_| Ok(())));
    pump(&mut f.app, "the prompt", |a| {
        a.sync.unlock.as_ref().is_some_and(|u| !u.paths.is_empty())
    });
    f.app.handle_key(key_esc()).unwrap();
    assert_eq!(f.app.mode, AppMode::Normal);
    pump(&mut f.app, "the round to end", |a| !a.sync.unlock_running);
    assert!(f.gate.get(&key).is_none());
}

fn key_esc() -> KeyEvent {
    key(KeyCode::Esc)
}
