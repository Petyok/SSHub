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

    // Navigate to the end (14 downs from Address)
    for _ in 0..14 {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::OsIcon);

    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::SessionLogging
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

/// An identity whose passphrase is already in the store, plus its id.
fn identity_with_stored_passphrase(app: &mut App, secret: &str) -> i64 {
    let created = app
        .store
        .create_identity(&crate::store::NewIdentity {
            name: "prod-key".into(),
            username: None,
            private_key: None,
            certificate: None,
            sort_order: 0,
            has_password: true,
        })
        .unwrap();
    app.password_store
        .set(&crate::credentials::identity_key(created.id), secret)
        .unwrap();
    app.reload_identities().unwrap();
    created.id
}

#[test]
pub(crate) fn identity_form_prefills_the_stored_passphrase() {
    let (mut app, _secrets) = test_app_with_secrets(vec![]);
    identity_with_stored_passphrase(&mut app, "s3cret");

    let identity = app
        .identities
        .iter()
        .find(|i| i.name == "prod-key")
        .expect("the identity we just created")
        .clone();
    app.enter_identity_form(Some(&identity)).unwrap();

    let form = app.identity_form.as_ref().unwrap();
    assert_eq!(
        form.password, "s3cret",
        "the field starts from what is stored"
    );
    assert_eq!(
        form.password_original, "s3cret",
        "and remembers it, so an untouched save is a no-op"
    );
    assert!(!form.password_revealed, "masked until asked");
}

#[test]
pub(crate) fn clearing_the_passphrase_removes_it_from_the_store() {
    let (mut app, secrets) = test_app_with_secrets(vec![]);
    let id = identity_with_stored_passphrase(&mut app, "s3cret");

    let identity = app
        .identities
        .iter()
        .find(|i| i.name == "prod-key")
        .expect("the identity we just created")
        .clone();
    app.enter_identity_form(Some(&identity)).unwrap();
    app.identity_form.as_mut().unwrap().password.clear();
    app.save_identity_form().unwrap();

    use crate::credentials::PasswordStore;
    assert_eq!(
        secrets.get(&crate::credentials::identity_key(id)).unwrap(),
        None,
        "an emptied field means the secret should go"
    );
    let saved = app
        .identities
        .iter()
        .find(|i| i.name == "prod-key")
        .unwrap();
    assert!(!saved.has_password, "and the row stops claiming it has one");
}

#[test]
pub(crate) fn saving_an_untouched_form_keeps_the_stored_passphrase() {
    let (mut app, secrets) = test_app_with_secrets(vec![]);
    let id = identity_with_stored_passphrase(&mut app, "s3cret");

    let identity = app
        .identities
        .iter()
        .find(|i| i.name == "prod-key")
        .expect("the identity we just created")
        .clone();
    app.enter_identity_form(Some(&identity)).unwrap();
    app.save_identity_form().unwrap();

    use crate::credentials::PasswordStore;
    assert_eq!(
        secrets.get(&crate::credentials::identity_key(id)).unwrap(),
        Some("s3cret".to_string())
    );
    let saved = app
        .identities
        .iter()
        .find(|i| i.name == "prod-key")
        .unwrap();
    assert!(saved.has_password);
}

#[test]
pub(crate) fn reveal_and_copy_binds_differ_and_never_echo_the_secret() {
    let (mut app, _secrets) = test_app_with_secrets(vec![]);
    identity_with_stored_passphrase(&mut app, "s3cret");
    let identity = app
        .identities
        .iter()
        .find(|i| i.name == "prod-key")
        .expect("the identity we just created")
        .clone();

    // Copy only: nothing is revealed, and the notice names the secret, not its value.
    app.enter_identity_form(Some(&identity)).unwrap();
    app.identity_form.as_mut().unwrap().field = IdentityFormField::Password;
    app.config
        .keybinds
        .set(KeyAction::CopySecret, vec!["F5".into()]);
    app.config
        .keybinds
        .set(KeyAction::RevealSecret, vec!["F6".into()]);

    app.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::empty()))
        .unwrap();
    assert!(!app.identity_form.as_ref().unwrap().password_revealed);
    let notice = app.identity_notice.clone().unwrap();
    assert!(notice.contains("passphrase"), "{notice}");
    assert!(!notice.contains("s3cret"), "the value must not be echoed");

    // Reveal: shows it, and walking off the field masks it again.
    app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::empty()))
        .unwrap();
    assert!(app.identity_form.as_ref().unwrap().password_revealed);
    app.identity_form_field_next();
    assert!(
        !app.identity_form.as_ref().unwrap().password_revealed,
        "leaving the field re-masks"
    );
}

#[test]
pub(crate) fn reveal_bind_is_ignored_away_from_the_secret_field() {
    let (mut app, _secrets) = test_app_with_secrets(vec![]);
    identity_with_stored_passphrase(&mut app, "s3cret");
    let identity = app
        .identities
        .iter()
        .find(|i| i.name == "prod-key")
        .expect("the identity we just created")
        .clone();
    app.enter_identity_form(Some(&identity)).unwrap();
    app.identity_form.as_mut().unwrap().field = IdentityFormField::Name;
    app.config
        .keybinds
        .set(KeyAction::RevealSecret, vec!["F6".into()]);

    app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::empty()))
        .unwrap();
    assert!(!app.identity_form.as_ref().unwrap().password_revealed);
}
