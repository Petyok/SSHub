use super::*;

impl App {
    pub(crate) fn edit_selected_host(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };
        let managed = self.hosts[host_idx].managed().cloned();
        match managed.as_ref().map(|m| m.source) {
            Some(HostSource::SshConfig) => {
                self.enter_host_form(managed.as_ref(), true)?;
            }
            Some(HostSource::Launcher) => {
                self.enter_host_form(managed.as_ref(), false)?;
            }
            None => {
                // Legacy ssh_config alias with no launcher row yet: materialize
                // it into launcher.db so it gains a group/identity/metadata
                // overlay, then edit that full form instead of the tags-only
                // HostDetail (which has no Group field).
                let name = self.hosts[host_idx].name().to_string();
                let materialized = crate::ssh::materialize_ssh_config_host(
                    self.resolver.as_ref(),
                    &self.store,
                    self.metadata.as_ref(),
                    &name,
                )?;
                if materialized {
                    self.reload_hosts()?;
                    self.restore_selection_by_name(&name);
                    let managed = self
                        .selected_host_index()
                        .and_then(|idx| self.hosts[idx].managed().cloned());
                    if managed.is_some() {
                        self.enter_host_form(managed.as_ref(), true)?;
                        return Ok(());
                    }
                }
                self.enter_host_detail()?;
            }
        }
        Ok(())
    }

    pub fn enter_host_form(
        &mut self,
        existing: Option<&ManagedHost>,
        metadata_only: bool,
    ) -> Result<()> {
        self.host_notice = None;
        self.groups = self.store.list_groups()?;
        if self.identities.is_empty() {
            self.identities = self.store.list_identities()?;
        }

        let default_identity_index = self
            .identities
            .iter()
            .position(|i| i.name == "Default")
            .unwrap_or(0);

        let form = if let Some(managed) = existing {
            let group_index = managed
                .group_id
                .and_then(|gid| self.groups.iter().position(|g| g.id == gid).map(|i| i + 1))
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
                identity_index,
                tags: managed.tags.join(", "),
                proxy_jump: managed.proxy_jump.clone().unwrap_or_default(),
                forward_agent: managed.forward_agent,
                remote_command: managed.remote_command.clone().unwrap_or_default(),
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
            let selected_group_id = self.selected_host_group_id();
            let group_index = selected_group_id
                .and_then(|gid| self.groups.iter().position(|g| g.id == gid).map(|i| i + 1))
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
                identity_index,
                tags: String::new(),
                proxy_jump: String::new(),
                forward_agent: false,
                remote_command: String::new(),
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

        let group_id = if form.group_index == 0 {
            None
        } else {
            self.groups.get(form.group_index - 1).map(|g| g.id)
        };
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
                    ..Default::default()
                },
            )?;
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
                    ..Default::default()
                },
            )?;
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
            })?;
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

    pub(crate) fn delete_selected_host(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };
        let Some(id) = self.hosts[host_idx].managed_id() else {
            self.host_notice = Some("Only launcher hosts can be deleted".into());
            return Ok(());
        };
        if self.hosts[host_idx].source() != HostSource::Launcher {
            self.host_notice = Some("Only launcher hosts can be deleted".into());
            return Ok(());
        }
        let name = self.hosts[host_idx].display_name().to_string();
        self.pending_delete = Some(PendingDelete::Host { id, name });
        self.mode = AppMode::ConfirmDelete;
        Ok(())
    }

    pub(crate) fn duplicate_selected_host(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };

        let copy_name = match &self.hosts[host_idx] {
            HostEntry::Managed(m) => {
                let Some(copy) = self.store.duplicate_host(m.id)? else {
                    self.host_notice = Some("Host not found".into());
                    return Ok(());
                };
                copy.name
            }
            HostEntry::Legacy { host, meta } => self.duplicate_legacy_to_launcher(host, meta)?,
        };

        self.reload_hosts()?;
        self.restore_selection_by_name(&copy_name);
        Ok(())
    }

    pub(crate) fn duplicate_legacy_to_launcher(
        &self,
        host: &SshHost,
        meta: &crate::metadata::HostMetadata,
    ) -> Result<String> {
        let mut name = format!("{}-copy", host.name);
        let mut suffix = 2u32;
        while self.store.get_host_by_name(&name)?.is_some() {
            name = format!("{}-copy-{}", host.name, suffix);
            suffix += 1;
        }

        let address = host.hostname.clone().unwrap_or_else(|| host.name.clone());
        let port = host.port.unwrap_or(22);

        let mut new_host = NewHost::launcher(name.clone(), address);
        new_host.port = port;
        new_host.tags = meta.tags.clone();
        new_host.notes = meta.description.clone();
        new_host.proxy_jump = host.proxy_jump.clone();
        new_host.forward_agent = host.forward_agent.unwrap_or(false);
        new_host.remote_command = host.remote_command.clone();
        new_host.identity_id = self.match_identity_for_ssh_host(host)?;
        self.store.create_host(&new_host)?;
        Ok(name)
    }

    pub(crate) fn match_identity_for_ssh_host(&self, host: &SshHost) -> Result<Option<i64>> {
        let user = host.user.as_deref();
        let key = host.identity_file.as_deref();
        if user.is_none() && key.is_none() {
            return Ok(None);
        }

        for identity in self.store.list_identities()? {
            let id_user = identity.username.as_deref();
            let id_key = identity
                .private_key
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned());
            let matches = match (user, key) {
                (Some(u), Some(k)) => id_user == Some(u) && id_key.as_deref() == Some(k),
                (Some(u), None) => id_user == Some(u),
                (None, Some(k)) => id_key.as_deref() == Some(k),
                (None, None) => false,
            };
            if matches {
                return Ok(Some(identity.id));
            }
        }

        let mut identity_name = format!("{}-identity", host.name);
        let mut suffix = 2u32;
        while self.store.get_identity_by_name(&identity_name)?.is_some() {
            identity_name = format!("{}-identity-{}", host.name, suffix);
            suffix += 1;
        }

        let created = self.store.create_identity(&NewIdentity {
            name: identity_name,
            username: host.user.clone(),
            private_key: key.map(PathBuf::from),
            ..Default::default()
        })?;
        Ok(Some(created.id))
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
            KeyCode::Enter if field == HostFormField::Group => self.open_field_picker(PickerKind::Group),
            KeyCode::Enter if field == HostFormField::Identity => {
                self.open_field_picker(PickerKind::Identity)
            }
            KeyCode::Enter if field == HostFormField::OsIcon => self.save_host_form()?,
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down if key.modifiers.is_empty() => {
                self.host_form_field_next();
            }
            KeyCode::BackTab | KeyCode::Up => self.host_form_field_prev(),
            KeyCode::Right if field.is_picker() || field.is_toggle() => {
                self.host_form_picker_scroll(1);
            }
            KeyCode::Left if field.is_picker() || field.is_toggle() => {
                self.host_form_picker_scroll(-1);
            }
            KeyCode::Char(' ')
                if key.modifiers.is_empty() && field == HostFormField::ForwardAgent =>
            {
                self.host_form_toggle();
            }
            KeyCode::Backspace => self.host_form_backspace(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control()
                    && !field.is_picker()
                    && !field.is_toggle() =>
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

    /// Number of selectable rows in the dropdown (incl. the "+ New group" row).
    pub fn field_picker_len(&self, kind: PickerKind) -> usize {
        match kind {
            // (none) + groups + "+ New group…"
            PickerKind::Group => self.groups.len() + 2,
            PickerKind::Identity => self.identities.len(),
        }
    }

    /// Index of the "+ New group…" row (Group picker only).
    pub(crate) fn field_picker_create_index(&self) -> usize {
        self.groups.len() + 1
    }

    pub(crate) fn open_field_picker(&mut self, kind: PickerKind) {
        let Some(form) = self.host_form.as_ref() else {
            return;
        };
        if form.metadata_only && kind == PickerKind::Identity {
            // Identity is a connection field for imported hosts — read-only.
            return;
        }
        let selected = match kind {
            PickerKind::Group => form.group_index,
            PickerKind::Identity => form.identity_index,
        };
        self.field_picker = Some(FieldPicker {
            kind,
            selected,
            creating: None,
            cursor: 0,
        });
        self.mode = AppMode::FieldPicker;
    }

    pub(crate) fn handle_key_field_picker(&mut self, key: KeyEvent) -> Result<()> {
        let Some(picker) = self.field_picker.as_ref() else {
            self.mode = AppMode::HostForm;
            return Ok(());
        };

        // Inline "create new group" text entry.
        if picker.creating.is_some() {
            return self.handle_key_field_picker_creating(key);
        }

        let kind = picker.kind;
        let len = self.field_picker_len(kind);
        match key.code {
            KeyCode::Esc => {
                self.field_picker = None;
                self.mode = AppMode::HostForm;
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                if let Some(p) = self.field_picker.as_mut() {
                    p.selected = (p.selected + 1) % len.max(1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                if let Some(p) = self.field_picker.as_mut() {
                    p.selected = (p.selected + len.saturating_sub(1)) % len.max(1);
                }
            }
            KeyCode::Enter => self.field_picker_confirm()?,
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn field_picker_confirm(&mut self) -> Result<()> {
        let Some(picker) = self.field_picker.as_ref() else {
            return Ok(());
        };
        match picker.kind {
            PickerKind::Group => {
                if picker.selected == self.field_picker_create_index() {
                    // Enter inline "new group" text entry.
                    if let Some(p) = self.field_picker.as_mut() {
                        p.creating = Some(String::new());
                        p.cursor = 0;
                    }
                    return Ok(());
                }
                let group_index = picker.selected;
                // Picking a group applies its default identity, if it has one.
                let default_identity_index = group_index
                    .checked_sub(1)
                    .and_then(|gi| self.groups.get(gi))
                    .and_then(|g| g.default_identity_id)
                    .and_then(|iid| self.identities.iter().position(|i| i.id == iid));
                if let Some(form) = self.host_form.as_mut() {
                    form.group_index = group_index;
                    if let Some(idx) = default_identity_index {
                        form.identity_index = idx;
                    }
                    form.dirty = true;
                }
            }
            PickerKind::Identity => {
                if let Some(form) = self.host_form.as_mut() {
                    form.identity_index = picker.selected;
                    form.dirty = true;
                }
            }
        }
        self.field_picker = None;
        self.mode = AppMode::HostForm;
        Ok(())
    }

    pub(crate) fn handle_key_field_picker_creating(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                // Back to the list, keep the dropdown open.
                if let Some(p) = self.field_picker.as_mut() {
                    p.creating = None;
                    p.cursor = 0;
                }
            }
            KeyCode::Enter => self.field_picker_create_group()?,
            KeyCode::Backspace => {
                if let Some(p) = self.field_picker.as_mut() {
                    if let Some(name) = p.creating.as_mut() {
                        let c = p.cursor;
                        p.cursor = text_input::backspace_at(name, c);
                    }
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(p) = self.field_picker.as_mut() {
                    if let Some(name) = p.creating.as_mut() {
                        let cur = p.cursor;
                        p.cursor = text_input::insert_at(name, cur, c);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn field_picker_create_group(&mut self) -> Result<()> {
        let name = self
            .field_picker
            .as_ref()
            .and_then(|p| p.creating.clone())
            .unwrap_or_default();
        let name = name.trim().to_string();
        if name.is_empty() {
            return Ok(());
        }
        // Reuse an existing group with the same name instead of erroring.
        let id = match self.store.list_groups()?.into_iter().find(|g| g.name == name) {
            Some(g) => g.id,
            None => {
                self.store
                    .create_group(&crate::store::NewHostGroup {
                        name: name.clone(),
                        sort_order: self.groups.len() as i32,
                        default_identity_id: None,
                    })?
                    .id
            }
        };
        self.groups = self.store.list_groups()?;
        if let Some(form) = self.host_form.as_mut() {
            // group_index: 0 = (none), 1.. = groups in list order.
            form.group_index = self
                .groups
                .iter()
                .position(|g| g.id == id)
                .map(|i| i + 1)
                .unwrap_or(0);
            form.dirty = true;
        }
        self.field_picker = None;
        self.mode = AppMode::HostForm;
        Ok(())
    }

    pub(crate) fn host_form_picker_scroll(&mut self, delta: i32) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        if !form.field.is_picker() && !form.field.is_toggle() {
            return;
        }
        if form.field == HostFormField::ForwardAgent {
            form.forward_agent = !form.forward_agent;
            form.dirty = true;
            return;
        }
        match form.field {
            HostFormField::Group => {
                let max = self.groups.len();
                let next = form.group_index as i32 + delta;
                form.group_index = next.clamp(0, max as i32) as usize;
                form.dirty = true;
            }
            HostFormField::Identity => {
                if !self.identities.is_empty() {
                    let max = self.identities.len() - 1;
                    let next = form.identity_index as i32 + delta;
                    form.identity_index = next.clamp(0, max as i32) as usize;
                    form.dirty = true;
                }
            }
            HostFormField::OsIcon => {
                let max = OS_ICON_OPTIONS.len().saturating_sub(1);
                let next = form.os_icon_index as i32 + delta;
                form.os_icon_index = next.clamp(0, max as i32) as usize;
                form.dirty = true;
            }
            _ => {}
        }
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

    pub(crate) fn enter_host_detail(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };
        let tags = self.hosts[host_idx].tags().join(", ");
        let description = self.hosts[host_idx]
            .description()
            .unwrap_or_default()
            .to_string();
        let environment = self.hosts[host_idx]
            .environment()
            .unwrap_or_default()
            .to_string();
        self.detail_edit = Some(HostDetailEdit {
            tags: tags.clone(),
            description,
            environment,
            field: DetailEditField::Tags,
            cursor: text_input::char_len(&tags),
        });
        self.mode = AppMode::HostDetail;
        Ok(())
    }

    pub(crate) fn cancel_host_detail(&mut self) -> Result<()> {
        if let Some(host_idx) = self.selected_host_index() {
            let host_name = self.hosts[host_idx].name().to_string();
            if let Some((_, meta)) = self.hosts[host_idx].legacy_mut() {
                if let Some(stored) = self.metadata.get(&host_name)? {
                    *meta = stored;
                }
            }
        }
        self.detail_edit = None;
        self.mode = AppMode::Normal;
        Ok(())
    }

    pub(crate) fn save_host_detail(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            self.detail_edit = None;
            self.mode = AppMode::Normal;
            return Ok(());
        };
        let Some(edit) = self.detail_edit.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        let host_name = self.hosts[host_idx].name().to_string();
        let favorite = self.hosts[host_idx].favorite();
        let last_connected = self.hosts[host_idx].last_connected();
        let description = optional_field(&edit.description);
        let environment = optional_field(&edit.environment);
        let tags = parse_tags(&edit.tags);

        // Managed hosts (launcher + imported ssh_config rows) live in
        // launcher.db — persist there, or the edit is lost on reload.
        if let HostEntry::Managed(managed) = &self.hosts[host_idx] {
            let id = managed.id;
            let update = crate::store::HostUpdate {
                tags: Some(tags),
                notes: Some(description),
                environment: Some(environment),
                ..Default::default()
            };
            if let Some(updated) = self.store.update_host(id, &update)? {
                self.hosts[host_idx] = HostEntry::Managed(updated);
            }
        } else {
            let meta = crate::metadata::HostMetadata {
                host_name: host_name.clone(),
                tags,
                description,
                environment,
                favorite,
                last_connected,
            };
            self.metadata.upsert(&meta)?;
            if let Some((_, stored_meta)) = self.hosts[host_idx].legacy_mut() {
                *stored_meta = meta;
            }
        }
        self.rebuild_filter();
        self.mode = AppMode::Normal;
        Ok(())
    }

    pub(crate) fn detail_edit_field_next(&mut self) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        edit.field = edit.field.next();
        edit.cursor = text_input::char_len(edit.active_field());
    }

    pub(crate) fn detail_edit_field_prev(&mut self) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        edit.field = edit.field.prev();
        edit.cursor = text_input::char_len(edit.active_field());
    }

    pub(crate) fn detail_edit_backspace(&mut self) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        let c = edit.cursor;
        edit.cursor = text_input::backspace_at(edit.active_field_mut(), c);
    }

    pub(crate) fn detail_edit_insert(&mut self, ch: char) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        let c = edit.cursor;
        edit.cursor = text_input::insert_at(edit.active_field_mut(), c, ch);
    }
}
