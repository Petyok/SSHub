//! An opened synced identity's key lives in a private ssh-agent and in memory:
//! ssh is pointed at the agent, SFTP authenticates from memory, and no private
//! key byte is written to any file.

use super::*;
use crate::credentials::{
    identity_key, synced_identity_agent, synced_key_for_public_file, SecretGate,
};
use std::process::Command;
use zeroize::Zeroizing;

fn have(tool: &str) -> bool {
    Command::new(tool).arg("-V").output().is_ok() || Command::new(tool).output().is_ok()
}

/// A fresh unencrypted ed25519 key as text; the files it was made in are gone.
fn generate_key() -> Option<String> {
    if !have("ssh-agent") || !have("ssh-keygen") || !have("ssh-add") {
        eprintln!("skipped: no OpenSSH client tools");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("k");
    let ok = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "t", "-f"])
        .arg(&file)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    std::fs::read_to_string(&file).ok()
}

fn host_with_keyless_identity(app: &App, id_name: &str) -> crate::store::ManagedHost {
    let identity = app
        .store
        .create_identity(&crate::store::NewIdentity {
            name: id_name.into(),
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

/// Every file under `root` (depth 4) that contains `needle`.
fn files_containing(
    root: &std::path::Path,
    needle: &str,
    depth: usize,
    out: &mut Vec<std::path::PathBuf>,
) {
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                files_containing(&p, needle, depth - 1, out);
            }
        } else if std::fs::read(&p)
            .map(|b| String::from_utf8_lossy(&b).contains(needle))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

#[test]
fn an_opened_synced_key_goes_to_a_private_agent_and_ssh_is_told_to_use_only_it() {
    let Some(pem) = generate_key() else { return };
    let app = test_app(vec![]);
    let host = host_with_keyless_identity(&app, "agent-id");
    let identity = host.identity.clone().unwrap();
    let ssh = crate::app::util::managed_to_ssh_host(&host);
    assert_eq!(
        (ssh.identity_file.clone(), ssh.identity_agent.clone()),
        (None, None)
    );

    let gate = SecretGate::default();
    gate.insert_private_key(&identity_key(identity.id), Zeroizing::new(pem.clone()))
        .unwrap();
    let agent = synced_identity_agent(identity.id).expect("registered");

    // The agent holds the key.
    let listed = Command::new("ssh-add")
        .arg("-L")
        .env("SSH_AUTH_SOCK", &agent.socket)
        .output()
        .unwrap();
    let public = std::fs::read_to_string(&agent.public_file).unwrap();
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains(public.trim()),
        "the agent lists the key the public file names"
    );
    assert!(
        !public.contains("PRIVATE"),
        "only the public half is a file"
    );

    // ssh is pointed at it and nothing else.
    let ssh = crate::app::util::managed_to_ssh_host(&host);
    assert_eq!(ssh.identity_agent.as_deref(), Some(agent.socket.as_path()));
    assert_eq!(ssh.identity_file.as_deref(), agent.public_file.to_str());
    let argv = crate::ssh::build_ssh_argv(&ssh);
    let sock_opt = format!("IdentityAgent={}", agent.socket.display());
    assert!(
        argv.contains(&sock_opt) && argv.contains(&"IdentitiesOnly=yes".to_string()),
        "{argv:?}"
    );

    // SFTP authenticates from memory with the same bytes.
    assert_eq!(
        synced_key_for_public_file(&agent.public_file)
            .as_deref()
            .map(String::as_str),
        Some(pem.as_str())
    );

    // No private key byte in any file of the temp or runtime directories.
    let body = pem.lines().nth(1).unwrap().to_string();
    let mut found = Vec::new();
    files_containing(&std::env::temp_dir(), &body, 3, &mut found);
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        files_containing(std::path::Path::new(&rt), &body, 3, &mut found);
    }
    assert!(found.is_empty(), "the key is in files: {found:?}");

    // Locking kills the agent and forgets the key.
    gate.lock_all();
    assert!(synced_identity_agent(identity.id).is_none());
    assert!(!agent.socket.exists(), "the agent's socket is gone");
    assert_eq!(
        crate::app::util::managed_to_ssh_host(&host).identity_agent,
        None
    );
}

#[test]
fn dropping_the_gate_stops_the_agent_and_a_non_identity_key_is_refused() {
    let Some(pem) = generate_key() else { return };
    let socket;
    {
        let gate = SecretGate::default();
        gate.insert_private_key(&identity_key(41), Zeroizing::new(pem.clone()))
            .unwrap();
        socket = synced_identity_agent(41).unwrap().socket;
        assert!(socket.exists());
        assert!(gate
            .insert_private_key("host:3", Zeroizing::new(pem.clone()))
            .is_err());
    }
    assert!(!socket.exists());
    assert!(synced_identity_agent(41).is_none());
}

#[test]
fn a_jump_host_with_a_synced_key_gets_its_own_block_in_a_config_that_keeps_the_users() {
    use std::os::unix::fs::PermissionsExt;
    let Some(pem) = generate_key() else { return };
    let gate = SecretGate::default();
    gate.insert_private_key(&identity_key(77), Zeroizing::new(pem))
        .unwrap();
    let agent = synced_identity_agent(77).unwrap();
    crate::credentials::note_hop_identity("hop.example", 77);

    let config = crate::credentials::hop_agent_config("me@hop.example:2222,other")
        .expect("the hop needs the private agent");
    assert!(
        config.starts_with(agent.socket.parent().unwrap()),
        "inside the agent's directory"
    );
    assert_eq!(
        std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains("Host hop.example"));
    assert!(text.contains(&format!("IdentityAgent {}", agent.socket.display())));
    assert!(text.contains(&format!("IdentityFile {}", agent.public_file.display())));
    assert!(text.contains("IdentitiesOnly yes"));
    // The user's own configuration still applies, after the hop's block.
    let (hop_at, own_at) = (
        text.find("Host hop.example").unwrap(),
        text.find("Include ~/.ssh/config").unwrap(),
    );
    assert!(hop_at < own_at);
    assert!(
        !text.contains("Host other"),
        "a hop without a synced key gets nothing"
    );

    // ssh reads it the way the jump command will.
    if let Ok(out) = Command::new("ssh")
        .args(["-G", "-F"])
        .arg(&config)
        .arg("hop.example")
        .output()
    {
        let shown = String::from_utf8_lossy(&out.stdout).to_lowercase();
        assert!(
            shown.contains(&format!("identityagent {}", agent.socket.display()).to_lowercase()),
            "{shown}"
        );
        assert!(shown.contains("identitiesonly yes"));
    }

    // The connection builder passes it.
    let mut ssh = crate::ssh::SshHost::new("dest");
    ssh.hostname = Some("dest.example".into());
    ssh.proxy_jump = Some("me@hop.example:2222".into());
    let argv = crate::ssh::build_ssh_argv(&ssh);
    let at = argv.iter().position(|a| a == "-F").expect("-F present");
    assert_eq!(argv[at + 1], config.display().to_string());

    // No hop to serve, no config; and locking removes the file with the agent.
    assert!(crate::credentials::hop_agent_config("unknown.example").is_none());
    gate.lock_all();
    assert!(!config.exists());
}
