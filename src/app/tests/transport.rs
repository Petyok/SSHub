use super::*;
use crate::session_transport::SessionTransport;
use crate::store::{HostSource, NewHost};

#[test]
pub(crate) fn session_argv_uses_mosh_when_transport_set() {
    let store = test_store();
    let mut nh = NewHost::launcher("edge", "10.0.0.1");
    nh.transport = SessionTransport::Mosh;
    nh.port = 2222;
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
    nh.transport = SessionTransport::Mosh;
    let managed = store.create_host(&nh).unwrap();
    let entry = HostEntry::from_managed(managed);

    let argv = session_argv_for_entry(&entry);
    assert_eq!(argv, vec!["mosh".to_string(), "cfg".to_string()]);
}
