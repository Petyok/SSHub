//! The "generate a new ed25519 key" form on the identities tab. Collects a name
//! and an optional passphrase, shells out to `ssh-keygen` (via
//! `crate::ssh::generate_ed25519`), writes the pair under the app data
//! directory and registers it as an identity.

use super::*;

/// Which field of [`KeygenForm`] currently has focus. The key type is fixed to
/// ed25519, so it is not an editable field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeygenField {
    #[default]
    Name = 0,
    Passphrase = 1,
}

impl KeygenField {
    pub fn next(self) -> Self {
        match self {
            KeygenField::Name => KeygenField::Passphrase,
            KeygenField::Passphrase => KeygenField::Name,
        }
    }

    pub fn prev(self) -> Self {
        // Two fields, so prev is the same cycle as next.
        self.next()
    }
}

/// State for the ed25519 key-generation popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeygenForm {
    pub name: String,
    pub passphrase: String,
    pub field: KeygenField,
    pub cursor: usize,
}

impl KeygenForm {
    /// Whether the name field currently has focus (for the renderer, which
    /// cannot name [`KeygenField`] across the module boundary).
    pub fn name_active(&self) -> bool {
        self.field == KeygenField::Name
    }

    /// Whether the passphrase field currently has focus.
    pub fn passphrase_active(&self) -> bool {
        self.field == KeygenField::Passphrase
    }

    fn active_field(&self) -> &str {
        match self.field {
            KeygenField::Name => &self.name,
            KeygenField::Passphrase => &self.passphrase,
        }
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.field {
            KeygenField::Name => &mut self.name,
            KeygenField::Passphrase => &mut self.passphrase,
        }
    }
}

impl App {
    pub(crate) fn enter_keygen_form(&mut self) {
        self.keygen_form = Some(KeygenForm {
            name: String::new(),
            passphrase: String::new(),
            field: KeygenField::Name,
            cursor: 0,
        });
        self.mode = AppMode::KeygenForm;
    }

    pub(crate) fn handle_key_keygen_form(&mut self, key: KeyEvent) -> Result<()> {
        if self.keygen_form.is_none() {
            return Ok(());
        }
        let field = self.keygen_form.as_ref().map(|f| f.field);
        match key.code {
            KeyCode::Esc => {
                self.keygen_form = None;
                self.mode = AppMode::Normal;
            }
            _ if self.is_save_key(&key) => self.save_keygen_form()?,
            // Enter on the last field saves; elsewhere it advances.
            KeyCode::Enter if field == Some(KeygenField::Passphrase) => {
                self.save_keygen_form()?;
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down if key.modifiers.is_empty() => {
                self.keygen_form_field_next();
            }
            KeyCode::BackTab | KeyCode::Up => self.keygen_form_field_prev(),
            KeyCode::Backspace => self.keygen_form_backspace(),
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End | KeyCode::Delete => {
                self.keygen_form_cursor_key(key.code)
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.keygen_form_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    fn keygen_form_field_next(&mut self) {
        if let Some(form) = self.keygen_form.as_mut() {
            form.field = form.field.next();
            form.cursor = text_input::char_len(form.active_field());
        }
    }

    fn keygen_form_field_prev(&mut self) {
        if let Some(form) = self.keygen_form.as_mut() {
            form.field = form.field.prev();
            form.cursor = text_input::char_len(form.active_field());
        }
    }

    fn keygen_form_backspace(&mut self) {
        if let Some(form) = self.keygen_form.as_mut() {
            let c = form.cursor;
            if c > 0 {
                form.cursor = text_input::backspace_at(form.active_field_mut(), c);
            }
        }
    }

    fn keygen_form_insert(&mut self, ch: char) {
        if let Some(form) = self.keygen_form.as_mut() {
            let c = form.cursor;
            form.cursor = text_input::insert_at(form.active_field_mut(), c, ch);
        }
    }

    fn keygen_form_cursor_key(&mut self, code: KeyCode) {
        if let Some(form) = self.keygen_form.as_mut() {
            let mut cursor = form.cursor;
            text_input::handle_cursor_key(code, form.active_field_mut(), &mut cursor);
            form.cursor = cursor;
        }
    }

    pub(crate) fn save_keygen_form(&mut self) -> Result<()> {
        let Some(form) = self.keygen_form.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        let name = form.name.trim().to_string();
        if name.is_empty() {
            self.identity_notice = Some("Key name is required".into());
            self.keygen_form = Some(form);
            return Ok(());
        }

        let dir = match crate::config::data_dir() {
            Ok(d) => d.join("keys"),
            Err(e) => {
                self.identity_notice = Some(format!("Cannot resolve data directory: {e:#}"));
                self.keygen_form = Some(form);
                return Ok(());
            }
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.identity_notice = Some(format!("Cannot create key directory: {e:#}"));
            self.keygen_form = Some(form);
            return Ok(());
        }

        let passphrase = form.passphrase.clone();
        let pass_opt = if passphrase.is_empty() {
            None
        } else {
            Some(passphrase.as_str())
        };

        match crate::ssh::generate_ed25519(&dir, &name, pass_opt) {
            Ok(out) => {
                let has_password = !passphrase.is_empty();
                let created = self.store.create_identity(&NewIdentity {
                    name: name.clone(),
                    username: None,
                    private_key: Some(out.private_key.clone()),
                    certificate: None,
                    sort_order: self.identities.len() as i32,
                    has_password,
                })?;
                if has_password {
                    self.password_store
                        .set(&crate::credentials::identity_key(created.id), &passphrase)
                        .ok();
                }
                self.reload_identities()?;
                if let Some(pos) = self.identities.iter().position(|i| i.name == name) {
                    self.identity_selected = pos;
                }
                self.mode = AppMode::Normal;
                self.identity_notice = Some(format!("Generated ed25519 key \"{name}\""));
                let _ = self.store.log_auth_event(
                    &name,
                    None,
                    "keygen",
                    "ok",
                    &format!("generated ed25519 key: {}", out.private_key.to_string_lossy()),
                );
            }
            Err(e) => {
                self.identity_notice = Some(format!("Key generation failed: {e:#}"));
                self.keygen_form = Some(form);
            }
        }
        Ok(())
    }
}
