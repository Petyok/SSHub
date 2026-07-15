use super::*;

impl App {
    pub fn refresh_agent_info(&mut self) {
        if self.agent_info_updated.elapsed() > std::time::Duration::from_secs(30) {
            self.agent_info = Some(crate::ssh::agent::detect_agent());
            self.agent_info_updated = std::time::Instant::now();
        }
    }

    pub(crate) fn switch_to_keys_tab(&mut self) -> Result<()> {
        self.active_tab = 3;
        self.reload_identities()?;
        self.agent_info_updated = std::time::Instant::now() - std::time::Duration::from_secs(60);
        self.refresh_agent_info();
        Ok(())
    }

    pub(crate) fn remove_selected_from_agent(&mut self) -> Result<()> {
        let Some(identity) = self.identities.get(self.identity_selected) else {
            return Ok(());
        };
        let Some(ref key_path) = identity.private_key else {
            self.identity_notice = Some("No private key path set".into());
            return Ok(());
        };
        let name = identity.name.clone();
        match crate::ssh::agent::remove_key(&key_path.to_string_lossy()) {
            Ok(()) => {
                self.identity_notice = Some(format!("Removed {} from agent", name));
                let _ = self.store.log_auth_event(
                    &name,
                    None,
                    "agent",
                    "ok",
                    &format!("key removed from agent: {}", key_path.to_string_lossy()),
                    None,
                );
                self.agent_info = None;
                self.agent_info_updated =
                    std::time::Instant::now() - std::time::Duration::from_secs(60);
                self.refresh_agent_info();
            }
            Err(e) => {
                self.identity_notice = Some(format!("Failed: {e:#}"));
                let _ = self.store.log_auth_event(
                    &name,
                    None,
                    "agent",
                    "fail",
                    &format!("remove from agent failed: {e:#}"),
                    None,
                );
            }
        }
        Ok(())
    }

    pub(crate) fn add_selected_to_agent(&mut self) -> Result<()> {
        let Some(identity) = self.identities.get(self.identity_selected) else {
            return Ok(());
        };
        let Some(ref key_path) = identity.private_key else {
            self.identity_notice = Some("No private key path set".into());
            return Ok(());
        };
        let name = identity.name.clone();
        match crate::ssh::agent::add_key(&key_path.to_string_lossy()) {
            Ok(()) => {
                self.identity_notice = Some(format!("Added {} to agent", name));
                let _ = self.store.log_auth_event(
                    &name,
                    None,
                    "agent",
                    "launched",
                    &format!("key added to agent: {}", key_path.to_string_lossy()),
                    None,
                );
                self.agent_info = None;
                self.agent_info_updated =
                    std::time::Instant::now() - std::time::Duration::from_secs(60);
                self.refresh_agent_info();
            }
            Err(e) => {
                self.identity_notice = Some(format!("Failed: {e:#}"));
                let _ = self.store.log_auth_event(
                    &name,
                    None,
                    "agent",
                    "fail",
                    &format!("add to agent failed: {e:#}"),
                    None,
                );
            }
        }
        Ok(())
    }

    pub(crate) fn handle_key_identity_form(&mut self, key: KeyEvent) -> Result<()> {
        let Some(form) = self.identity_form.as_ref() else {
            return Ok(());
        };
        let field = form.field;
        match key.code {
            KeyCode::Esc => self.cancel_identity_form()?,
            _ if self.is_save_key(&key) => self.save_identity_form()?,
            // Single-step model: Enter on the last field saves.
            KeyCode::Enter if field == IdentityFormField::Password => {
                self.save_identity_form()?;
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down if key.modifiers.is_empty() => {
                self.identity_form_field_next();
            }
            KeyCode::BackTab | KeyCode::Up => self.identity_form_field_prev(),
            KeyCode::Backspace => self.identity_form_backspace(),
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End | KeyCode::Delete => {
                self.identity_form_cursor_key(key.code)
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.identity_form_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn reload_identities(&mut self) -> Result<()> {
        let selected_name = self
            .identities
            .get(self.identity_selected)
            .map(|i| i.name.clone());
        self.identities = self.store.list_identities()?;
        if let Some(name) = selected_name {
            if let Some(pos) = self.identities.iter().position(|i| i.name == name) {
                self.identity_selected = pos;
            } else {
                self.clamp_identity_selected();
            }
        } else {
            self.clamp_identity_selected();
        }
        Ok(())
    }

    pub(crate) fn enter_identity_form(&mut self, existing: Option<&Identity>) -> Result<()> {
        let form = if let Some(identity) = existing {
            IdentityFormEdit {
                id: Some(identity.id),
                name: identity.name.clone(),
                username: identity.username.clone().unwrap_or_default(),
                private_key: identity
                    .private_key
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                certificate: identity
                    .certificate
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                password: String::new(),
                has_password: identity.has_password,
                pasted_key: None,
                field: IdentityFormField::Name,
                cursor: text_input::char_len(&identity.name),
                editing: true,
                edit_snapshot: String::new(),
                dirty: false,
            }
        } else {
            IdentityFormEdit {
                id: None,
                name: String::new(),
                username: String::new(),
                private_key: String::new(),
                certificate: String::new(),
                password: String::new(),
                has_password: false,
                pasted_key: None,
                field: IdentityFormField::Name,
                cursor: 0,
                editing: true,
                edit_snapshot: String::new(),
                dirty: false,
            }
        };
        self.identity_form = Some(form);
        self.mode = AppMode::IdentityForm;
        Ok(())
    }

    pub(crate) fn edit_selected_identity(&mut self) -> Result<()> {
        let Some(identity) = self.selected_identity().cloned() else {
            return Ok(());
        };
        self.enter_identity_form(Some(&identity))
    }

    pub(crate) fn delete_selected_identity(&mut self) -> Result<()> {
        let Some(identity) = self.selected_identity().cloned() else {
            return Ok(());
        };
        self.pending_delete = Some(PendingDelete::Identity {
            id: identity.id,
            name: identity.name.clone(),
        });
        self.mode = AppMode::ConfirmDelete;
        Ok(())
    }

    pub(crate) fn cancel_identity_form(&mut self) -> Result<()> {
        if self.identity_form.as_ref().is_some_and(|f| f.dirty) {
            self.mode = AppMode::ConfirmDiscard;
        } else {
            self.discard_identity_form()?;
        }
        Ok(())
    }

    pub(crate) fn discard_identity_form(&mut self) -> Result<()> {
        self.identity_form = None;
        self.mode = AppMode::Normal;
        Ok(())
    }

    pub(crate) fn save_identity_form(&mut self) -> Result<()> {
        let Some(form) = self.identity_form.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        let name = form.name.trim();
        if name.is_empty() {
            self.identity_notice = Some("Identity name is required".into());
            self.identity_form = Some(form);
            return Ok(());
        }

        let username = optional_field(&form.username);
        let private_key = if let Some(blob) = form.pasted_key.as_deref() {
            match crate::ssh::write_key_material(name, blob) {
                Ok(path) => Some(path),
                Err(e) => {
                    self.identity_notice = Some(format!("Could not write key file: {e}"));
                    self.identity_form = Some(form);
                    return Ok(());
                }
            }
        } else {
            optional_path(&form.private_key)
        };
        let certificate = optional_path(&form.certificate);

        // If the key is passphrase-protected, require (and verify) the
        // passphrase before saving — otherwise auto-auth would silently fail
        // later. Skip when a passphrase is already stored (has_password).
        if let Some(ref key_path) = private_key {
            let expanded = crate::ssh::expand_tilde(&key_path.to_string_lossy());
            if form.password.is_empty() && !form.has_password {
                if crate::ssh::key_is_encrypted(&expanded) == Some(true) {
                    self.identity_notice =
                        Some("This key is passphrase-protected — enter its passphrase".into());
                    let mut form = form;
                    form.field = IdentityFormField::Password;
                    form.cursor = 0;
                    self.identity_form = Some(form);
                    return Ok(());
                }
            } else if !form.password.is_empty()
                && crate::ssh::passphrase_matches(&expanded, &form.password) == Some(false)
            {
                self.identity_notice = Some("Passphrase does not match this key".into());
                self.identity_form = Some(form);
                return Ok(());
            }
        }

        let password_changed = !form.password.is_empty();
        let new_has_password = if password_changed {
            true
        } else {
            form.has_password
        };

        if let Some(id) = form.id {
            if password_changed {
                if let Err(e) = self
                    .password_store
                    .set(&crate::credentials::identity_key(id), &form.password)
                {
                    self.identity_notice =
                        Some(format!("Saved, but storing the passphrase failed: {e}"));
                }
            }
            self.store.update_identity(
                id,
                &IdentityUpdate {
                    name: Some(name.to_string()),
                    username: Some(username),
                    private_key: Some(private_key),
                    certificate: Some(certificate),
                    has_password: Some(new_has_password),
                    ..Default::default()
                },
            )?;
        } else {
            let sort_order = self.identities.len() as i32;
            let created = self.store.create_identity(&NewIdentity {
                name: name.to_string(),
                username,
                private_key,
                certificate,
                sort_order,
                has_password: new_has_password,
            })?;
            if password_changed {
                if let Err(e) = self.password_store.set(
                    &crate::credentials::identity_key(created.id),
                    &form.password,
                ) {
                    self.identity_notice =
                        Some(format!("Saved, but storing the passphrase failed: {e}"));
                }
            }
        }

        self.mode = AppMode::Normal;
        self.reload_identities()?;
        if let Some(pos) = self.identities.iter().position(|i| i.name == name) {
            self.identity_selected = pos;
        }
        Ok(())
    }

    pub(crate) fn identity_form_field_next(&mut self) {
        let Some(form) = self.identity_form.as_mut() else {
            return;
        };
        form.field = form.field.next();
        form.cursor = text_input::char_len(form.active_field());
    }

    pub(crate) fn identity_form_field_prev(&mut self) {
        let Some(form) = self.identity_form.as_mut() else {
            return;
        };
        form.field = form.field.prev();
        form.cursor = text_input::char_len(form.active_field());
    }

    pub(crate) fn identity_form_backspace(&mut self) {
        let Some(form) = self.identity_form.as_mut() else {
            return;
        };
        if form.field == IdentityFormField::PrivateKey && form.pasted_key.is_some() {
            // One backspace discards the pasted blob entirely.
            form.pasted_key = None;
            form.private_key.clear();
            form.cursor = 0;
            return;
        }
        let c = form.cursor;
        if c > 0 {
            form.cursor = text_input::backspace_at(form.active_field_mut(), c);
            form.dirty = true;
        }
    }

    pub(crate) fn identity_form_insert(&mut self, ch: char) {
        let Some(form) = self.identity_form.as_mut() else {
            return;
        };
        form.clear_pasted_key_marker();
        let c = form.cursor;
        form.cursor = text_input::insert_at(form.active_field_mut(), c, ch);
        form.dirty = true;
    }

    fn identity_form_cursor_key(&mut self, code: KeyCode) {
        if let Some(form) = self.identity_form.as_mut() {
            // Mirror the backspace guard: one Delete on a pasted key blob discards
            // it whole, rather than mangling the placeholder char-by-char.
            if code == KeyCode::Delete
                && form.field == IdentityFormField::PrivateKey
                && form.pasted_key.is_some()
            {
                form.pasted_key = None;
                form.private_key.clear();
                form.cursor = 0;
                return;
            }
            let mut cursor = form.cursor;
            let changed = text_input::handle_cursor_key(code, form.active_field_mut(), &mut cursor);
            form.cursor = cursor;
            if changed == Some(true) {
                form.dirty = true;
            }
        }
    }

    /// Columns in the identities grid — the exact value the renderer uses.
    pub(crate) fn identity_cards_per_row(&self) -> i32 {
        let inner_w = crate::tui::screens::keys::inner_width(self.terminal_area.width);
        crate::tui::screens::keys::resolve_columns(inner_w, self.config.appearance.identity_columns)
            as i32
    }

    /// Change how many columns the identities grid shows (`delta` +1/-1),
    /// clamped to what fits, and persist it.
    pub(crate) fn adjust_identity_columns(&mut self, delta: i32) {
        let inner_w = crate::tui::screens::keys::inner_width(self.terminal_area.width);
        let max = crate::tui::screens::keys::max_columns(inner_w) as i32;
        // Start from the currently-shown count so +/- feels direct even when
        // the stored preference is 0 (auto).
        let current = self.identity_cards_per_row();
        let next = (current + delta).clamp(1, max);
        self.config.appearance.identity_columns = next as usize;
        self.save_config_quietly();
        self.identity_notice = Some(format!("Identity columns: {next}"));
    }

    /// Grid move: `dr` rows down/up, `dc` columns right/left. Left/right never
    /// wrap across rows so navigation stays predictable.
    pub(crate) fn move_identity_grid(&mut self, dr: i32, dc: i32) {
        if self.identities.is_empty() {
            self.identity_selected = 0;
            return;
        }
        let cpr = self.identity_cards_per_row();
        let len = self.identities.len() as i32;
        let cur = self.identity_selected as i32;
        if dc != 0 {
            let col = cur % cpr;
            let target_col = col + dc;
            if target_col < 0 || target_col >= cpr {
                return; // stay put at the row edge
            }
            let next = cur + dc;
            if next >= 0 && next < len {
                self.identity_selected = next as usize;
            }
        } else if dr != 0 {
            let mut next = cur + dr * cpr;
            // Moving down past the end: drop onto the (shorter) last row's card.
            if dr > 0 && next >= len && cur < len - 1 {
                next = len - 1;
            }
            if next >= 0 && next < len {
                self.identity_selected = next as usize;
            }
        }
    }

    pub(crate) fn clamp_identity_selected(&mut self) {
        if self.identities.is_empty() {
            self.identity_selected = 0;
        } else if self.identity_selected >= self.identities.len() {
            self.identity_selected = self.identities.len() - 1;
        }
    }

    pub fn selected_identity(&self) -> Option<&Identity> {
        self.identities.get(self.identity_selected)
    }
}
