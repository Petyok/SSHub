use super::*;
use crate::session_transport::SessionTransport;
use crate::store::{HostSource, NewHost};

#[test]
pub(crate) fn session_argv_uses_mosh_when_transport_set() {
    let store = test_store();
    let mut nh = NewHost::launcher("edge", "10.0.0.1");
    nh.transport = Some(SessionTransport::Mosh);
    nh.port = Some(2222);
    let managed = store.create_host(&nh).unwrap();
    let entry = HostEntry::from_managed(managed);

    let argv = session_argv_for_entry(&entry);
    assert_eq!(argv.first().map(String::as_str), Some("mosh"));
    assert!(argv.iter().any(|a| a.starts_with("--ssh=")));
    assert!(argv.iter().any(|a| a.contains("10.0.0.1")));
}

#[test]
pub(crate) fn session_argv_stays_ssh_by_default() {
    let store = test_store();
    let managed = store
        .create_host(&NewHost::launcher("web", "example.com"))
        .unwrap();
    let entry = HostEntry::from_managed(managed);

    let argv = session_argv_for_entry(&entry);
    assert_eq!(argv.first().map(String::as_str), Some("ssh"));
}

#[test]
pub(crate) fn legacy_host_mosh_uses_alias_argv() {
    let mut host = host("roam");
    host.hostname = Some("roam.example".into());
    let mut entry = HostEntry::new(host);
    legacy_meta(&mut entry).transport = SessionTransport::Mosh;

    let argv = session_argv_for_entry(&entry);
    assert_eq!(argv, vec!["mosh".to_string(), "roam".to_string()]);
}

#[test]
pub(crate) fn ssh_config_managed_mosh_uses_alias() {
    let store = test_store();
    let mut nh = NewHost::launcher("cfg", "ignored");
    nh.source = HostSource::SshConfig;
    nh.transport = Some(SessionTransport::Mosh);
    let managed = store.create_host(&nh).unwrap();
    let entry = HostEntry::from_managed(managed);

    let argv = session_argv_for_entry(&entry);
    assert_eq!(argv, vec!["mosh".to_string(), "cfg".to_string()]);
}

#[test]
pub(crate) fn mosh_managed_accept_new_when_stored_secret() {
    let store = test_store();
    let mut nh = NewHost::launcher("edge", "10.0.0.1");
    nh.transport = Some(SessionTransport::Mosh);
    nh.port = Some(2222);
    let entry = HostEntry::from_managed(store.create_host(&nh).unwrap());

    let argv = prepare_session_connect_argv(session_argv_for_entry(&entry), true);
    assert_eq!(argv.first().map(String::as_str), Some("mosh"));
    assert!(
        argv.iter()
            .any(|a| a.contains("accept-new") && a.starts_with("--ssh=")),
        "expected --ssh=… to include accept-new, got {argv:?}"
    );
}

#[test]
pub(crate) fn mosh_alias_accept_new_when_stored_secret() {
    let mut host = host("roam");
    host.hostname = Some("roam.example".into());
    let mut entry = HostEntry::new(host);
    legacy_meta(&mut entry).transport = SessionTransport::Mosh;

    let argv = prepare_session_connect_argv(session_argv_for_entry(&entry), true);
    assert_eq!(
        argv,
        vec![
            "mosh".to_string(),
            "--ssh=ssh -o StrictHostKeyChecking=accept-new".to_string(),
            "roam".to_string(),
        ]
    );
}

#[test]
pub(crate) fn mosh_skips_accept_new_without_stored_secret() {
    let store = test_store();
    let mut nh = NewHost::launcher("edge", "10.0.0.1");
    nh.transport = Some(SessionTransport::Mosh);
    let entry = HostEntry::from_managed(store.create_host(&nh).unwrap());

    let argv = prepare_session_connect_argv(session_argv_for_entry(&entry), false);
    assert!(!argv.iter().any(|a| a.contains("accept-new")));
}

#[test]
pub(crate) fn connect_argv_uses_group_inherited_values() {
    // Oracle: hand-computed argv. The host sets nothing; the group carries
    // username/port/ProxyJump/forwarding, so the connect argv must be built
    // from the resolved values — exactly this word list, in this order
    // (see `build_ssh_argv`: -p, -J, -o ForwardAgent, then user@host).
    let store = test_store();
    let group = store
        .create_group(&crate::store::NewHostGroup {
            name: "prod".into(),
            default_username: Some("deploy".into()),
            default_port: Some(2222),
            default_proxy_jump: Some("bastion".into()),
            default_transport: Some(SessionTransport::Ssh),
            default_forward_agent: Some(true),
            ..Default::default()
        })
        .unwrap();
    let mut nh = NewHost::launcher("web", "10.0.0.1");
    nh.group_id = Some(group.id);
    nh.port = None;
    nh.forward_agent = None;
    nh.transport = None;
    let managed = store.create_host(&nh).unwrap();

    let resolved = store.resolve_connection(&managed).unwrap();
    assert_eq!(resolved.port, 2222);
    assert_eq!(resolved.username.as_deref(), Some("deploy"));
    let argv = resolved_session_argv(&managed, &resolved);
    assert_eq!(
        argv,
        vec![
            "ssh",
            "-p",
            "2222",
            "-J",
            "bastion",
            "-o",
            "ForwardAgent=yes",
            "deploy@10.0.0.1",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>(),
    );
}

#[test]
pub(crate) fn connect_argv_expands_launcher_only_jump_hops() {
    // A hop naming a host that exists only in launcher.db is unknown to ssh
    // (`ssh -J bastion-eu` looks the name up in DNS / ~/.ssh/config and fails),
    // so connect must hand ssh `user@address:port`. The first hop also brings
    // a launcher hop's own jump (jump-2 via bastion-eu) is never followed, since
    // ssh cannot see it; unknown tokens stay as typed.
    let store = test_store();
    let mut eu = NewHost::launcher("bastion-eu", "10.20.0.2");
    eu.username = Some("ops".into());
    store.create_host(&eu).unwrap();
    let mut j2 = NewHost::launcher("jump-2", "10.20.9.2");
    j2.username = Some("ops".into());
    j2.port = Some(2200);
    j2.proxy_jump = Some("bastion-eu".into());
    store.create_host(&j2).unwrap();
    let mut db = NewHost::launcher("db-1", "10.20.4.11");
    db.proxy_jump = Some("jump-2,dns.example.com".into());
    let managed = store.create_host(&db).unwrap();

    let resolved = store.resolve_connection(&managed).unwrap();
    let want = "ops@10.20.9.2:2200,dns.example.com";
    assert_eq!(resolved.jump_argv.as_deref(), Some(want));
    // The stored / displayed / exported value is untouched.
    assert_eq!(
        resolved.proxy_jump.as_deref(),
        Some("jump-2,dns.example.com")
    );
    let argv = resolved_session_argv(&managed, &resolved);
    let at = argv.iter().position(|a| a == "-J").unwrap();
    assert_eq!(argv[at + 1], want);

    // Oracle: real ssh accepts the expanded jump list and reports it back.
    if std::process::Command::new("ssh").arg("-V").output().is_ok() {
        let out = std::process::Command::new("ssh")
            .args(["-F", "/dev/null", "-G"])
            .args(&argv[1..])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.lines().any(|l| l == format!("proxyjump {want}")),
            "ssh -G did not report the jump: {text}"
        );
    }
}

#[test]
pub(crate) fn a_bad_stored_jump_hop_is_a_notice_not_an_error_that_ends_the_tui() {
    // Regression: `resolve_connection` refuses a hop ssh must not be handed
    // (leading `-`); connect and push key propagated that with `?` to the event
    // loop, which ended the TUI. The user must get a notice instead.
    let mut app = crate::app::tests::test_app(vec![]);
    let mut nh = NewHost::launcher("victim", "10.0.0.9");
    nh.proxy_jump = Some("-oProxyCommand=id".into());
    let managed = app.store().create_host(&nh).unwrap();
    app.reload_hosts().unwrap();
    let entry = HostEntry::from_managed(managed);

    assert!(app.connect_host_entry(entry.clone()).is_ok());
    assert!(app.sessions.is_empty(), "nothing may be spawned");
    let notice = app.host_notice.clone().unwrap_or_default();
    assert!(
        notice.contains("victim") && notice.contains("not a valid"),
        "{notice}"
    );

    // Push key: a real (generated) key so the failure is the hop, not the key.
    app.host_notice = None;
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("id");
    assert!(std::process::Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-N", "", "-q", "-f", key.to_str().unwrap()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false));
    let identity = crate::store::Identity {
        id: 1,
        name: "k".into(),
        username: None,
        private_key: Some(key),
        certificate: None,
        has_password: false,
    };
    assert!(app.push_public_key_to_host(&entry, &identity).is_ok());
    assert!(app.sessions.is_empty());
    let notice = app.host_notice.clone().unwrap_or_default();
    assert!(notice.contains("not a valid"), "{notice}");
}

#[test]
pub(crate) fn an_expanded_hop_never_injects_a_second_hop_from_the_database() {
    // A saved hop whose stored address is `evil,other` would, written verbatim
    // into `-J`, add a second jump host. Sending the hop's name instead would let
    // ssh resolve it to some other machine, so such a host is refused.
    let store = test_store();
    for (name, addr, user) in [
        ("comma", "evil,other", None),
        ("space", "a b", None),
        ("at", "10.0.0.1", Some("a@b")),
        ("fine", "10.0.0.7", Some("ops")),
    ] {
        let mut h = NewHost::launcher(name, addr);
        h.username = user.map(String::from);
        store.create_host(&h).unwrap();
    }
    for bad in ["comma", "space"] {
        let mut db = NewHost::launcher(format!("db-{bad}"), "10.0.0.5");
        db.proxy_jump = Some(bad.into());
        let managed = store.create_host(&db).unwrap();
        assert!(store.resolve_connection(&managed).is_err(), "{bad} refused");
    }
    // ssh splits the user on the last `@`: `a@b` as a user is one valid hop.
    let mut db = NewHost::launcher("db-at", "10.0.0.5");
    db.proxy_jump = Some("at".into());
    let managed = store.create_host(&db).unwrap();
    let r = store.resolve_connection(&managed).unwrap();
    assert_eq!(r.jump_argv.as_deref(), Some("a@b@10.0.0.1"));
    let mut db = NewHost::launcher("db-fine", "10.0.0.5");
    db.proxy_jump = Some("fine".into());
    let managed = store.create_host(&db).unwrap();
    let r = store.resolve_connection(&managed).unwrap();
    assert_eq!(r.jump_argv.as_deref(), Some("ops@10.0.0.7"));
}

#[test]
pub(crate) fn fallback_paths_refuse_a_host_with_an_invalid_jump_instead_of_going_direct() {
    // Tunnel, broadcast and SFTP all fall back past `resolve_connection` when
    // it fails. None of them may then connect without the jump.
    let mut app = crate::app::tests::test_app(vec![]);
    let mut nh = NewHost::launcher("victim", "10.0.0.9");
    nh.proxy_jump = Some("-oProxyCommand=id".into());
    let victim = app.store().create_host(&nh).unwrap();
    let tunnel_id = app
        .store()
        .create_tunnel(&crate::store::NewTunnel {
            host_id: Some(victim.id),
            tunnel_type: crate::store::TunnelType::Local,
            local_port: 18080,
            remote_host: "localhost".into(),
            remote_port: 80,
            label: None,
            auto_connect: false,
        })
        .unwrap();
    app.reload_hosts().unwrap();
    app.reload_tunnels().unwrap();

    // Tunnel: refused with the reason, nothing running.
    app.toggle_tunnel_by_id(tunnel_id).unwrap();
    let notice = app.tunnel_notice.clone().unwrap_or_default();
    assert!(notice.contains("not a valid"), "{notice}");
    assert!(!app.tunnel_manager.is_running(tunnel_id));
    assert!(!app.tunnel_manager.has_child(tunnel_id));

    // Broadcast: the host is left out, and the user is told why.
    app.open_broadcast_for_hosts(vec![victim.id], "victim only".into());
    assert!(app.broadcast_setup.is_none(), "no candidates, no wizard");
    let notice = app.host_notice.clone().unwrap_or_default();
    assert!(
        notice.contains("victim") && notice.contains("not a valid"),
        "{notice}"
    );

    // SFTP: the host keeps a jump, so the existing no-SFTP-over-ProxyJump
    // refusal fires instead of a direct session.
    let entry = HostEntry::from_managed(victim);
    let host = crate::app::sftp_ssh_host(app.resolver.as_ref(), app.store(), &entry);
    assert!(host.proxy_jump.is_some());
}

#[test]
pub(crate) fn a_chain_resolves_each_hops_own_stored_secret_and_never_the_targets() {
    use crate::credentials::PasswordStore;
    use crate::session::auth_prompt::PromptClass;
    // you -> hop-1 -> hop-2 -> dest-1, all saved hosts with a stored password.
    let (app, secrets) = crate::app::tests::test_app_with_secrets(vec![]);
    let mk = |name: &str, addr: &str, user: &str, jump: Option<&str>| {
        let mut h = NewHost::launcher(name, addr);
        h.username = Some(user.into());
        h.proxy_jump = jump.map(String::from);
        app.store().create_host(&h).unwrap()
    };
    let lb1 = mk("hop-1", "192.0.2.11", "ops", None);
    let lb2 = mk("hop-2", "192.0.2.12", "ops", None);
    let target = mk("dest-1", "192.0.2.91", "ops", Some("hop-1,hop-2"));
    mk("hop-3", "192.0.2.13", "ops", None);
    for (host, pw) in [
        (&lb1, "hop-1-pw"),
        (&lb2, "hop-2-pw"),
        (&target, "dest-1-pw"),
    ] {
        secrets
            .set(&crate::credentials::host_key(host.id), pw)
            .unwrap();
    }
    let mut app = app;
    app.reload_hosts().unwrap();

    let resolved = app.store().resolve_connection(&target).unwrap();
    let jump = resolved.proxy_jump.as_deref();
    let staged = hop_secrets(app.store(), &app.hosts, secrets.as_ref(), jump);
    // References only: no stored secret is read, nor copied anywhere.
    let view: Vec<(&str, &str, &[crate::credentials::SecretRef])> = staged
        .iter()
        .map(|h| {
            (
                h.user.as_deref().unwrap_or(""),
                h.host.as_str(),
                h.refs.as_slice(),
            )
        })
        .collect();
    let key = |id| {
        [crate::credentials::SecretRef::Key(
            crate::credentials::host_key(id),
        )]
    };
    assert_eq!(
        view,
        [
            ("ops", "192.0.2.11", key(lb1.id).as_slice()),
            ("ops", "192.0.2.12", key(lb2.id).as_slice())
        ]
    );
    assert!(!format!("{staged:?}").contains("-pw"));
    // A saved hop with nothing stored (key auth) is not staged at all, so
    // callers keep BatchMode and strict host-key checking for key-only chains.
    let only_lb3 = hop_secrets(app.store(), &app.hosts, secrets.as_ref(), Some("hop-3"));
    assert!(only_lb3.is_empty());
    let mixed = hop_secrets(
        app.store(),
        &app.hosts,
        secrets.as_ref(),
        Some("hop-1,hop-3"),
    );
    assert_eq!(mixed.len(), 1);
    assert!(staged.iter().all(|h| h.class == PromptClass::Password));

    // Lazy form: nothing is read until a hop asks, and then only that hop's.
    let creds: Vec<_> = hop_specs(app.store(), &app.hosts, jump.unwrap())
        .into_iter()
        .map(|s| {
            s.lazy(std::sync::Arc::new(crate::app::tests::SharedStore(
                secrets.clone(),
            )))
        })
        .collect();
    let lb2_cred = crate::session::hop_auth::find(&creds, "ops@192.0.2.12's password: ").unwrap();
    assert_eq!(lb2_cred.name, "hop-2");
    assert_eq!(lb2_cred.host_id, Some(lb2.id));
    assert_eq!(
        lb2_cred.secret(PromptClass::Password).unwrap().value(),
        "hop-2-pw"
    );
    // The target is no hop: its prompt names nobody in the chain.
    assert!(crate::session::hop_auth::find(&creds, "ops@192.0.2.91's password: ").is_none());

    // A hop written as its address is still that host; a raw hop is nobody.
    let by_address = hop_specs(app.store(), &app.hosts, "ops@192.0.2.11:22,ops@192.0.2.1");
    assert_eq!(by_address.len(), 1);
}

#[test]
pub(crate) fn connecting_through_a_chain_arms_the_session_with_each_hops_own_credential() {
    // Nothing listens on port 1: the session only has to exist and carry its
    // hops. ssh is killed when the app drops.
    let mut app = crate::app::tests::test_app(vec![]);
    let mut hosts = Vec::new();
    for (name, jump) in [
        ("hop-1", None),
        ("hop-2", None),
        ("dest-1", Some("hop-1,hop-2")),
    ] {
        let mut h = NewHost::launcher(name, "127.0.0.1");
        h.port = Some(1);
        h.username = Some("ops".into());
        h.proxy_jump = jump.map(String::from);
        hosts.push(app.store().create_host(&h).unwrap());
    }
    app.reload_hosts().unwrap();
    let entry = HostEntry::from_managed(hosts[2].clone());
    app.connect_host_entry(entry).unwrap();
    let session = app.sessions.last().expect("a session was spawned");
    let names: Vec<&str> = session
        .auth
        .as_ref()
        .expect("an ssh session has the interactive channel")
        .hops
        .iter()
        .map(|h| h.name.as_str())
        .collect();
    assert_eq!(names, ["hop-1", "hop-2"]);
}
