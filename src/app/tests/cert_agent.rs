use super::*;
use std::process::Command;

/// Point `PATH` at a fake `ssh-add` that appends its argv to `recorder`.
/// Returns the tempdir (kept alive by the caller) and the previous `PATH`
/// for restoration.
fn fake_ssh_add(recorder: &std::path::Path) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("ssh-add");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}\nprintf '%s\\n' '---' >> {}\n",
            recorder.display(),
            recorder.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let old_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{old_path}", dir.path().display()));
    (dir, old_path)
}

/// Real keypair + cert from the real `ssh-keygen` (oracle, not fixtures).
fn make_pair(dir: &tempfile::TempDir, stem: &str, key_id: &str) -> (String, String) {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        panic!("ssh-keygen required for cert oracle tests");
    }
    let ca = dir.path().join(format!("{stem}-ca"));
    let key = dir.path().join(stem);
    assert!(Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-f"])
        .arg(&ca)
        .args(["-N", "", "-C", "test ca"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-f"])
        .arg(&key)
        .args(["-N", "", "-C", "test key"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("ssh-keygen")
        .arg("-s")
        .arg(&ca)
        .arg("-I")
        .arg(key_id)
        .arg("-n")
        .arg("alice")
        .arg("-V")
        .arg("20240101000000:20300101000000")
        .arg(format!("{}.pub", key.display()))
        .output()
        .unwrap()
        .status
        .success());
    let key_s = key.to_string_lossy().into_owned();
    let cert_s = format!("{key_s}-cert.pub");
    assert!(std::path::Path::new(&cert_s).exists());
    (key_s, cert_s)
}

fn select_identity(app: &mut App, name: &str) {
    app.reload_identities().unwrap();
    app.identity_selected = app
        .identities
        .iter()
        .position(|i| i.name == name)
        .expect("identity present");
}

#[test]
fn agent_add_passes_cert_alongside_key() {
    // Oracle: the fake `ssh-add` records the real argv the app spawned.
    let _lock = crate::test_env::lock_home();
    let old_sock = std::env::var("SSH_AUTH_SOCK").ok();
    std::env::remove_var("SSH_AUTH_SOCK");
    let record_dir = tempfile::tempdir().unwrap();
    let recorder = record_dir.path().join("argv.txt");
    let (_fake_dir, old_path) = fake_ssh_add(&recorder);
    let fixtures = tempfile::tempdir().unwrap();
    let (key, cert) = make_pair(&fixtures, "id", "k1");

    let mut app = test_app(vec![]);
    app.store
        .create_identity(&crate::store::NewIdentity {
            name: "cert-id".into(),
            username: None,
            private_key: Some(std::path::PathBuf::from(&key)),
            certificate: Some(std::path::PathBuf::from(&cert)),
            sort_order: 0,
            has_password: false,
        })
        .unwrap();
    select_identity(&mut app, "cert-id");
    app.add_selected_to_agent().unwrap();

    assert_eq!(
        app.identity_notice.as_deref().unwrap_or(""),
        "Added cert-id to agent"
    );
    let recorded = std::fs::read_to_string(&recorder).unwrap();
    assert_eq!(recorded, format!("{key}\n{cert}\n---\n"));

    std::env::set_var("PATH", old_path);
    match old_sock {
        Some(v) => std::env::set_var("SSH_AUTH_SOCK", v),
        None => std::env::remove_var("SSH_AUTH_SOCK"),
    }
}

#[test]
fn agent_add_warns_when_cert_does_not_match_key() {
    // Oracle: two independent real keypairs, crossed — the warning must name
    // the mismatch instead of printing a plain success.
    let _lock = crate::test_env::lock_home();
    let old_sock = std::env::var("SSH_AUTH_SOCK").ok();
    std::env::remove_var("SSH_AUTH_SOCK");
    let record_dir = tempfile::tempdir().unwrap();
    let recorder = record_dir.path().join("argv.txt");
    let (_fake_dir, old_path) = fake_ssh_add(&recorder);
    let fixtures = tempfile::tempdir().unwrap();
    let (key_a, _cert_a) = make_pair(&fixtures, "a", "ka");
    let (_key_b, cert_b) = make_pair(&fixtures, "b", "kb");

    let mut app = test_app(vec![]);
    app.store
        .create_identity(&crate::store::NewIdentity {
            name: "crossed".into(),
            username: None,
            private_key: Some(std::path::PathBuf::from(&key_a)),
            certificate: Some(std::path::PathBuf::from(&cert_b)),
            sort_order: 0,
            has_password: false,
        })
        .unwrap();
    select_identity(&mut app, "crossed");
    app.add_selected_to_agent().unwrap();

    assert!(
        app.identity_notice
            .as_deref()
            .unwrap_or("")
            .contains("does not match"),
        "notice: {:?}",
        app.identity_notice
    );
    std::env::set_var("PATH", old_path);
    match old_sock {
        Some(v) => std::env::set_var("SSH_AUTH_SOCK", v),
        None => std::env::remove_var("SSH_AUTH_SOCK"),
    }
}
