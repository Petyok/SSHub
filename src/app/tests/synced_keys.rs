//! An opened synced identity's key reaches ssh as a 0600 file on tmpfs, and
//! only while the gate holds it.

use super::*;
use crate::credentials::{identity_key, synced_identity_key_file, SecretGate};
use std::os::unix::fs::PermissionsExt;
use zeroize::Zeroizing;

const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n";

fn host_with_keyless_identity(app: &App) -> crate::store::ManagedHost {
    let identity = app
        .store
        .create_identity(&crate::store::NewIdentity {
            name: "work".into(),
            username: Some("me".into()),
            private_key: None,
            certificate: None,
            sort_order: 0,
            has_password: false,
        })
        .unwrap();
    let host = app
        .store
        .create_host(&crate::store::NewHost {
            identity_id: Some(identity.id),
            ..crate::store::NewHost::launcher("web", "10.0.0.1")
        })
        .unwrap();
    app.store.get_host(host.id).unwrap().unwrap()
}

#[test]
fn an_opened_synced_key_becomes_the_identity_file_until_the_gate_locks() {
    let app = test_app(vec![]);
    let host = host_with_keyless_identity(&app);
    let identity = host.identity.clone().unwrap();
    let ssh = crate::app::util::managed_to_ssh_host(&host);
    assert_eq!(ssh.identity_file, None, "nothing to give ssh yet");

    let dir = tempfile::tempdir().unwrap();
    let gate = SecretGate::default();
    gate.insert_private_key_in(
        dir.path(),
        &identity_key(identity.id),
        Zeroizing::new(KEY.into()),
    )
    .unwrap();
    let path = synced_identity_key_file(identity.id).expect("registered");
    assert!(path.starts_with(dir.path()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), KEY);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let ssh = crate::app::util::managed_to_ssh_host(&host);
    assert_eq!(ssh.identity_file.as_deref(), path.to_str());

    gate.lock_all();
    assert!(!path.exists(), "the key file goes with the lock");
    assert!(synced_identity_key_file(identity.id).is_none());
    assert_eq!(
        crate::app::util::managed_to_ssh_host(&host).identity_file,
        None
    );
}

#[test]
fn dropping_the_gate_removes_its_key_files_and_a_non_identity_key_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path;
    {
        let gate = SecretGate::default();
        gate.insert_private_key_in(dir.path(), &identity_key(41), Zeroizing::new(KEY.into()))
            .unwrap();
        path = synced_identity_key_file(41).unwrap();
        assert!(path.exists());
        assert!(gate
            .insert_private_key_in(dir.path(), "host:3", Zeroizing::new(KEY.into()))
            .is_err());
    }
    assert!(!path.exists());
}
