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
