use super::*;

#[test]
pub(crate) fn duplicate_legacy_preserves_session_logging_override() {
    let mut app = test_app(vec![("legacy-web", host("legacy-web"))]);
    legacy_meta(&mut app.hosts[0]).session_logging = crate::session_log::SessionLoggingOverride::On;

    let (ssh_host, meta) = match &app.hosts[0] {
        HostEntry::Legacy { host, meta } => (host.clone(), meta.clone()),
        _ => panic!("expected legacy host"),
    };

    let copy_name =
        crate::hosts::duplicate_legacy_to_launcher(&app.store, &ssh_host, &meta).unwrap();
    let created = app
        .store
        .get_host_by_name(&copy_name)
        .unwrap()
        .expect("duplicate host row");
    assert_eq!(
        created.session_logging,
        crate::session_log::SessionLoggingOverride::On
    );
}
