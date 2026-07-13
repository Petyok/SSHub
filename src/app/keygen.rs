use super::*;

impl App {
    pub(crate) fn enter_keygen_form(&mut self) -> Result<()> {
        let form = KeygenFormEdit {
            key_type: KeygenType::Ed25519,
            passphrase: String::new(),
            comment: String::new(),
            target_path: "~/.ssh/id_ed25519_sshub".to_string(),
            field: KeygenFormField::KeyType,
            cursor: 0,
            dirty: false,
        };
        self.keygen_form = Some(form);
        self.keygen_notice = None;
        self.mode = AppMode::KeygenForm;
        Ok(())
    }

    pub(crate) fn cancel_keygen_form(&mut self) -> Result<()> {
        if self.keygen_form.as_ref().is_some_and(|f| f.dirty) {
            self.mode = AppMode::ConfirmDiscard;
        } else {
            self.discard_keygen_form()?;
        }
        Ok(())
    }

    pub(crate) fn discard_keygen_form(&mut self) -> Result<()> {
        self.keygen_form = None;
        self.mode = AppMode::Normal;
        Ok(())
    }

    pub(crate) fn save_keygen_form(&mut self) -> Result<()> {
        let Some(form) = self.keygen_form.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        let target_path_str = form.target_path.trim();
        if target_path_str.is_empty() {
            self.keygen_notice = Some("Target path is required".into());
            self.keygen_form = Some(form);
            return Ok(());
        }

        let expanded_path = crate::ssh::expand_tilde(target_path_str);
        let path = std::path::Path::new(&expanded_path);

        // Run key generation
        let key_type_str = match form.key_type {
            KeygenType::Ed25519 => "ed25519",
            KeygenType::Rsa4096 => "rsa",
        };
        let bits = match form.key_type {
            KeygenType::Ed25519 => None,
            KeygenType::Rsa4096 => Some(4096),
        };

        if let Err(e) = crate::ssh::generate_key_pair(
            key_type_str,
            bits,
            &form.passphrase,
            &form.comment,
            path,
        ) {
            self.keygen_notice = Some(format!("Generation failed: {e:#}"));
            self.keygen_form = Some(form);
            return Ok(());
        }

        // Create the identity so it appears in the tab immediately
        let filename = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "id_generated".to_string());

        let identity_name = if !form.comment.trim().is_empty() {
            form.comment.trim().to_string()
        } else {
            filename
        };

        let has_password = !form.passphrase.is_empty();
        let sort_order = self.identities.len() as i32;

        let created_result = self.store.create_identity(&crate::store::NewIdentity {
            name: identity_name.clone(),
            username: None,
            private_key: Some(std::path::PathBuf::from(target_path_str)),
            certificate: None,
            sort_order,
            has_password,
        });

        match created_result {
            Ok(created) => {
                if has_password {
                    if let Err(e) = self.password_store.set(
                        &crate::credentials::identity_key(created.id),
                        &form.passphrase,
                    ) {
                        self.identity_notice = Some(format!(
                            "Key generated, but storing the passphrase failed: {e}"
                        ));
                    }
                }
                self.identity_notice =
                    Some(format!("Generated key and identity '{}'", identity_name));
            }
            Err(e) => {
                self.identity_notice = Some(format!(
                    "Key generated at {}, but failed to save identity: {e}",
                    target_path_str
                ));
            }
        }

        self.mode = AppMode::Normal;
        self.reload_identities()?;
        if let Some(pos) = self.identities.iter().position(|i| i.name == identity_name) {
            self.identity_selected = pos;
        }
        Ok(())
    }

    pub(crate) fn keygen_form_field_next(&mut self) {
        let Some(form) = self.keygen_form.as_mut() else {
            return;
        };
        form.field = form.field.next();
        form.cursor = text_input::char_len(form.active_field());
    }

    pub(crate) fn keygen_form_field_prev(&mut self) {
        let Some(form) = self.keygen_form.as_mut() else {
            return;
        };
        form.field = form.field.prev();
        form.cursor = text_input::char_len(form.active_field());
    }

    pub(crate) fn keygen_form_backspace(&mut self) {
        let Some(form) = self.keygen_form.as_mut() else {
            return;
        };
        if form.field == KeygenFormField::KeyType {
            return;
        }
        let c = form.cursor;
        if c > 0 {
            if let Some(field) = form.active_field_mut() {
                form.cursor = text_input::backspace_at(field, c);
                form.dirty = true;
            }
        }
    }

    pub(crate) fn keygen_form_insert(&mut self, ch: char) {
        let Some(form) = self.keygen_form.as_mut() else {
            return;
        };
        if form.field == KeygenFormField::KeyType {
            return;
        }
        let c = form.cursor;
        if let Some(field) = form.active_field_mut() {
            form.cursor = text_input::insert_at(field, c, ch);
            form.dirty = true;
        }
    }

    fn keygen_form_cursor_key(&mut self, code: KeyCode) {
        if let Some(form) = self.keygen_form.as_mut() {
            if form.field == KeygenFormField::KeyType {
                return;
            }
            let mut cursor = form.cursor;
            if let Some(field) = form.active_field_mut() {
                let changed = text_input::handle_cursor_key(code, field, &mut cursor);
                form.cursor = cursor;
                if changed == Some(true) {
                    form.dirty = true;
                }
            }
        }
    }

    pub(crate) fn keygen_form_cycle_type(&mut self, _delta: i32) {
        let Some(form) = self.keygen_form.as_mut() else {
            return;
        };
        if form.field != KeygenFormField::KeyType {
            return;
        }

        let old_type = form.key_type;
        form.key_type = match old_type {
            KeygenType::Ed25519 => KeygenType::Rsa4096,
            KeygenType::Rsa4096 => KeygenType::Ed25519,
        };

        // Auto-update path if it's still the default path
        if old_type == KeygenType::Ed25519 && form.target_path == "~/.ssh/id_ed25519_sshub" {
            form.target_path = "~/.ssh/id_rsa_sshub".to_string();
        } else if old_type == KeygenType::Rsa4096 && form.target_path == "~/.ssh/id_rsa_sshub" {
            form.target_path = "~/.ssh/id_ed25519_sshub".to_string();
        }

        form.dirty = true;
    }

    pub(crate) fn handle_key_keygen_form(&mut self, key: KeyEvent) -> Result<()> {
        let Some(form) = self.keygen_form.as_ref() else {
            return Ok(());
        };
        let field = form.field;
        match key.code {
            KeyCode::Esc => self.cancel_keygen_form()?,
            _ if self.is_save_key(&key) => self.save_keygen_form()?,
            KeyCode::Enter if field == KeygenFormField::TargetPath => {
                self.save_keygen_form()?;
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down if key.modifiers.is_empty() => {
                self.keygen_form_field_next();
            }
            KeyCode::BackTab | KeyCode::Up => self.keygen_form_field_prev(),
            KeyCode::Left | KeyCode::Right if field == KeygenFormField::KeyType => {
                let delta = if key.code == KeyCode::Left { -1 } else { 1 };
                self.keygen_form_cycle_type(delta);
            }
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
}
