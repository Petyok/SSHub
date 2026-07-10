use super::*;

#[test]
pub(crate) fn multiline_paste_into_form_stays_in_field() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.active_tab = 3; // keys tab
    app.enter_identity_form(None).unwrap();
    assert_eq!(app.mode, AppMode::IdentityForm);

    // Navigate to the Private key path field.
    while app.identity_form.as_ref().unwrap().field != IdentityFormField::PrivateKey {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }

    // Paste a multi-line PEM blob. Previously the newlines fired
    // Enter/save and the rest ran as commands; now it must all stay put.
    let key_blob =
        "-----BEGIN OPENSSH PRIVATE KEY-----\nabc123\ndef456\n-----END OPENSSH PRIVATE KEY-----\n";
    app.handle_paste(key_blob).unwrap();

    // Still in the form, on the same field, no host connection triggered.
    assert_eq!(app.mode, AppMode::IdentityForm);
    let form = app.identity_form.as_ref().unwrap();
    assert_eq!(form.field, IdentityFormField::PrivateKey);
    // Key material captured as a blob (written to a file on save).
    assert_eq!(form.pasted_key.as_deref(), Some(key_blob));
    assert!(form.private_key.contains("pasted key"));
}

#[test]
pub(crate) fn host_form_up_down_navigate_fields_in_both_directions() {
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Address
    );

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Password
    );

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Username
    );

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Label);

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Username
    );

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Password
    );

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Address
    );

    // Navigate to the end (12 downs from Address)
    for _ in 0..12 {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::OsIcon);

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::RemoteCommand
    );
}

#[test]
pub(crate) fn host_form_picker_at_boundary_moves_to_adjacent_field() {
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    for _ in 0..6 {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Group);

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Port);

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Group);

    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Identity
    );
}
