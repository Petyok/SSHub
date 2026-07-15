use super::*;

impl App {
    pub fn enter_host_form(
        &mut self,
        existing: Option<&ManagedHost>,
        metadata_only: bool,
    ) -> Result<()> {
        self.host_notice = None;
        self.load_groups()?;
        if self.identities.is_empty() {
            self.identities = self.store.list_identities()?;
        }

        let default_identity_index = self
            .identities
            .iter()
            .position(|i| i.name == "Default")
            .unwrap_or(0);

        let form = if let Some(managed) = existing {
            // Multi-select: pre-check every non-reserved group the host belongs
            // to (Favorites is excluded — it's toggled via `f`).
            let group_ids: std::collections::BTreeSet<i64> = managed
                .groups
                .iter()
                .filter(|g| !g.reserved)
                .map(|g| g.id)
                .collect();
            // Highlight the host's primary group's row in the dropdown.
            let group_index = managed
                .group_id
                .and_then(|gid| self.groups.iter().position(|g| g.id == gid))
                .unwrap_or(0);
            let identity_index = managed
                .identity_id
                .and_then(|iid| self.identities.iter().position(|i| i.id == iid))
                .unwrap_or(default_identity_index);

            let start_field = if metadata_only {
                HostFormField::Label
            } else {
                HostFormField::Address
            };
            let start_cursor = if metadata_only {
                text_input::char_len(managed.label.as_deref().unwrap_or(""))
            } else {
                text_input::char_len(&managed.address)
            };

            HostFormEdit {
                id: Some(managed.id),
                address: managed.address.clone(),
                username: managed
                    .username
                    .clone()
                    .or_else(|| managed.identity.as_ref().and_then(|i| i.username.clone()))
                    .unwrap_or_default(),
                label: managed.label.clone().unwrap_or_default(),
                name: managed.name.clone(),
                port: managed.port.to_string(),
                group_index,
                group_ids,
                identity_index,
                tags: managed.tags.join(", "),
                proxy_jump: managed.proxy_jump.clone().unwrap_or_default(),
                forward_agent: managed.forward_agent,
                remote_command: managed.remote_command.clone().unwrap_or_default(),
                session_logging: managed.session_logging,
                os_icon_index: os_icon_index_from_option(&managed.os_icon),
                password: String::new(),
                has_password: managed.has_password,
                field: start_field,
                cursor: start_cursor,
                metadata_only,
                editing: true,
                edit_snapshot: String::new(),
                dirty: false,
            }
        } else {
            // Prefill group + identity from the group the user is currently in.
            // A new host added inside a group inherits the group's default identity.
            // Only prefill from a real (non-reserved) group; a selection under
            // Favorites must not pre-check anything.
            let selected_group_id = self
                .selected_host_group_id()
                .filter(|gid| self.groups.iter().any(|g| g.id == *gid));
            let group_ids: std::collections::BTreeSet<i64> =
                selected_group_id.into_iter().collect();
            let group_index = selected_group_id
                .and_then(|gid| self.groups.iter().position(|g| g.id == gid))
                .unwrap_or(0);
            let identity_index = selected_group_id
                .and_then(|gid| self.groups.iter().find(|g| g.id == gid))
                .and_then(|g| g.default_identity_id)
                .and_then(|iid| self.identities.iter().position(|i| i.id == iid))
                .unwrap_or(default_identity_index);

            HostFormEdit {
                id: None,
                address: String::new(),
                username: String::new(),
                label: String::new(),
                name: String::new(),
                port: "22".into(),
                group_index,
                group_ids,
                identity_index,
                tags: String::new(),
                proxy_jump: String::new(),
                forward_agent: false,
                remote_command: String::new(),
                session_logging: crate::session_log::SessionLoggingOverride::Inherit,
                os_icon_index: 0,
                password: String::new(),
                has_password: false,
                field: HostFormField::Address,
                cursor: 0,
                metadata_only: false,
                editing: true,
                edit_snapshot: String::new(),
                dirty: false,
            }
        };

        self.host_form = Some(form);
        self.mode = AppMode::HostForm;
        Ok(())
    }

    pub(crate) fn cancel_host_form(&mut self) -> Result<()> {
        if self.host_form.as_ref().is_some_and(|f| f.dirty) {
            self.mode = AppMode::ConfirmDiscard;
        } else {
            self.discard_host_form()?;
        }
        Ok(())
    }

    pub(crate) fn discard_host_form(&mut self) -> Result<()> {
        self.host_form = None;
        self.mode = AppMode::Normal;
        Ok(())
    }

    pub(crate) fn save_host_form(&mut self) -> Result<()> {
        let Some(form) = self.host_form.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        // Multi-select groups: the authoritative set is `form.group_ids`,
        // ordered here by the group list order. The primary `group_id` (first
        // one) is still threaded through NewHost/HostUpdate so single-group
        // reads stay consistent; `set_host_groups` writes the full set below.
        let group_ids: Vec<i64> = self
            .groups
            .iter()
            .filter(|g| form.group_ids.contains(&g.id))
            .map(|g| g.id)
            .collect();
        let group_id = group_ids.first().copied();
        let identity_id = self.identities.get(form.identity_index).map(|i| i.id);
        let tags = parse_tags(&form.tags);
        let label = optional_field(&form.label);
        let host_pw_changed = !form.password.is_empty();
        let new_has_password = if host_pw_changed {
            true
        } else {
            form.has_password
        };
        let username = optional_field(&form.username);

        if form.metadata_only {
            let Some(id) = form.id else {
                self.mode = AppMode::Normal;
                return Ok(());
            };
            let saved_name = form.name.clone();
            if host_pw_changed {
                if let Err(e) = self
                    .password_store
                    .set(&crate::credentials::host_key(id), &form.password)
                {
                    self.host_notice = Some(format!("Saved, but storing the password failed: {e}"));
                }
            }
            self.store.update_host(
                id,
                &HostUpdate {
                    label: Some(label),
                    group_id: Some(group_id),
                    identity_id: Some(identity_id),
                    tags: Some(tags),
                    has_password: Some(new_has_password),
                    username: Some(username.clone()),
                    session_logging: Some(form.session_logging),
                    ..Default::default()
                },
            )?;
            self.store.set_host_groups(id, &group_ids)?;
            self.mode = AppMode::Normal;
            self.reload_hosts()?;
            self.restore_selection_by_name(&saved_name);
            return Ok(());
        }

        let address = form.address.trim();
        let name = form.name.trim();
        if address.is_empty() {
            self.host_notice = Some("Address is required".into());
            self.host_form = Some(form);
            return Ok(());
        }
        if name.is_empty() {
            self.host_notice = Some("Name (alias) is required".into());
            self.host_form = Some(form);
            return Ok(());
        }

        let port: u16 = match form.port.trim().parse() {
            Ok(p) if p > 0 => p,
            _ => {
                self.host_notice = Some("Port must be a positive number".into());
                self.host_form = Some(form);
                return Ok(());
            }
        };

        let os_icon = os_icon_from_index(form.os_icon_index);
        let proxy_jump = optional_field(&form.proxy_jump);
        let remote_command = optional_field(&form.remote_command);

        // Avoid the `hosts.name` UNIQUE constraint (which would otherwise abort
        // the app): if the name is taken, fall back to `name-2`, `name-3`, …
        // An edit keeps its own current name via `exclude_id`.
        let unique_name = self.store.unique_host_name(name, form.id)?;
        if unique_name != name {
            self.host_notice = Some(format!(
                "Name '{name}' already exists \u{2014} saved as '{unique_name}'"
            ));
        }
        let name = unique_name.as_str();
        let saved_name = name.to_string();
        if let Some(id) = form.id {
            if host_pw_changed {
                if let Err(e) = self
                    .password_store
                    .set(&crate::credentials::host_key(id), &form.password)
                {
                    self.host_notice = Some(format!("Saved, but storing the password failed: {e}"));
                }
            }
            self.store.update_host(
                id,
                &HostUpdate {
                    name: Some(name.to_string()),
                    label: Some(label),
                    address: Some(address.to_string()),
                    port: Some(port),
                    group_id: Some(group_id),
                    identity_id: Some(identity_id),
                    os_icon: Some(os_icon),
                    tags: Some(tags),
                    proxy_jump: Some(proxy_jump),
                    forward_agent: Some(form.forward_agent),
                    remote_command: Some(remote_command),
                    has_password: Some(new_has_password),
                    username: Some(username),
                    session_logging: Some(form.session_logging),
                    ..Default::default()
                },
            )?;
            self.store.set_host_groups(id, &group_ids)?;
        } else {
            let created = self.store.create_host(&NewHost {
                name: name.to_string(),
                label,
                address: address.to_string(),
                port,
                group_id,
                identity_id,
                os_icon,
                tags,
                notes: None,
                proxy_jump,
                forward_agent: form.forward_agent,
                remote_command,
                source: HostSource::Launcher,
                has_password: new_has_password,
                username,
                session_logging: form.session_logging,
            })?;
            self.store.set_host_groups(created.id, &group_ids)?;
            if host_pw_changed {
                if let Err(e) = self
                    .password_store
                    .set(&crate::credentials::host_key(created.id), &form.password)
                {
                    self.host_notice = Some(format!("Saved, but storing the password failed: {e}"));
                }
            }
        }

        self.mode = AppMode::Normal;
        self.reload_hosts()?;
        self.restore_selection_by_name(&saved_name);
        Ok(())
    }

    pub(crate) fn handle_key_host_form(&mut self, key: KeyEvent) -> Result<()> {
        let Some(form) = self.host_form.as_ref() else {
            return Ok(());
        };
        let field = form.field;
        match key.code {
            KeyCode::Esc => self.cancel_host_form()?,
            _ if self.is_save_key(&key) => self.save_host_form()?,
            // Single-step model: type straight into the active field.
            // Enter/Tab/Down advance; Enter on the LAST field saves the form
            // (a modifier-free save path; F2/Ctrl+S always work).
            KeyCode::Enter if field == HostFormField::Group => {
                self.open_field_picker(PickerKind::Group)
            }
            KeyCode::Enter if field == HostFormField::Identity => {
                self.open_field_picker(PickerKind::Identity)
            }
            KeyCode::Enter if field == HostFormField::OsIcon => self.save_host_form()?,
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down if key.modifiers.is_empty() => {
                self.host_form_field_next();
            }
            KeyCode::BackTab | KeyCode::Up => self.host_form_field_prev(),
            KeyCode::Right if field.is_picker() || field.is_toggle() || field.is_tri_state() => {
                self.host_form_picker_scroll(1);
            }
            KeyCode::Left if field.is_picker() || field.is_toggle() || field.is_tri_state() => {
                self.host_form_picker_scroll(-1);
            }
            // Text fields: move the edit cursor within the field.
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End | KeyCode::Delete => {
                self.host_form_cursor_key(key.code)
            }
            KeyCode::Char(' ')
                if key.modifiers.is_empty()
                    && (field == HostFormField::ForwardAgent
                        || field == HostFormField::SessionLogging) =>
            {
                self.host_form_toggle();
            }
            KeyCode::Backspace => self.host_form_backspace(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control()
                    && !field.is_picker()
                    && !field.is_toggle()
                    && !field.is_tri_state() =>
            {
                self.host_form_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn host_form_field_next(&mut self) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        form.field = form.field.next();
        form.cursor = text_input::char_len(form.active_field());
    }

    pub(crate) fn host_form_field_prev(&mut self) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        form.field = form.field.prev();
        form.cursor = text_input::char_len(form.active_field());
    }

    pub(crate) fn host_form_toggle(&mut self) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        if form.metadata_only && form.field.is_connection_field() {
            return;
        }
        if form.field == HostFormField::ForwardAgent {
            form.forward_agent = !form.forward_agent;
            form.dirty = true;
        } else if form.field == HostFormField::SessionLogging {
            form.session_logging = form.session_logging.next();
            form.dirty = true;
        }
    }

    pub(crate) fn host_form_backspace(&mut self) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        if form.metadata_only && form.field.is_connection_field() {
            return;
        }
        if form.field.is_picker() || form.field.is_toggle() {
            return;
        }
        let c = form.cursor;
        if c > 0 {
            form.cursor = text_input::backspace_at(form.active_field_mut(), c);
            form.dirty = true;
        }
    }

    pub(crate) fn host_form_insert(&mut self, ch: char) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        if form.metadata_only && form.field.is_connection_field() {
            return;
        }
        if form.field.is_picker() || form.field.is_toggle() {
            return;
        }
        let c = form.cursor;
        form.cursor = text_input::insert_at(form.active_field_mut(), c, ch);
        form.dirty = true;
    }

    fn host_form_cursor_key(&mut self, code: KeyCode) {
        if let Some(form) = self.host_form.as_mut() {
            if form.field.is_picker() || form.field.is_toggle() {
                return;
            }
            if code == KeyCode::Delete && form.metadata_only && form.field.is_connection_field() {
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
}
