use super::*;

impl App {
    pub(crate) fn switch_to_tunnels_tab(&mut self) -> Result<()> {
        self.active_tab = 2;
        self.reload_tunnels()?;
        Ok(())
    }

    pub(crate) fn handle_key_tunnels(&mut self, key: KeyEvent) -> Result<()> {
        self.tunnel_notice = None;

        if self.try_tab_switch(&key)? {
            return Ok(());
        }

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::MoveDown, &key) => {
                if !self.tunnels.is_empty() {
                    self.tunnel_selected = (self.tunnel_selected + 1).min(self.tunnels.len() - 1);
                }
            }
            _ if self.is_action(KeyAction::MoveUp, &key) => {
                self.tunnel_selected = self.tunnel_selected.saturating_sub(1);
            }
            _ if self.is_action(KeyAction::AddHost, &key) => {
                self.tunnel_form = Some(TunnelFormEdit {
                    editing_id: None,
                    tunnel_type: crate::store::TunnelType::Local,
                    local_port: String::new(),
                    remote_host: "localhost".into(),
                    remote_port: String::new(),
                    host_id: None,
                    label: String::new(),
                    auto_connect: false,
                    active_field: TunnelFormField::Host,
                    editing: true,
                    edit_snapshot: String::new(),
                    dirty: false,
                    cursor: 0,
                });
                self.mode = AppMode::TunnelForm;
            }
            _ if self.is_action(KeyAction::Edit, &key) => {
                if let Some(tunnel) = self.tunnels.get(self.tunnel_selected) {
                    self.tunnel_form = Some(TunnelFormEdit {
                        editing_id: Some(tunnel.id),
                        tunnel_type: tunnel.tunnel_type,
                        local_port: tunnel.local_port.to_string(),
                        remote_host: tunnel.remote_host.clone(),
                        remote_port: tunnel.remote_port.to_string(),
                        host_id: tunnel.host_id,
                        label: tunnel.label.clone().unwrap_or_default(),
                        auto_connect: tunnel.auto_connect,
                        active_field: TunnelFormField::Host,
                        editing: true,
                        edit_snapshot: String::new(),
                        dirty: false,
                        cursor: 0,
                    });
                    self.mode = AppMode::TunnelForm;
                }
            }
            _ if self.is_action(KeyAction::Delete, &key) => {
                if let Some(tunnel) = self.tunnels.get(self.tunnel_selected) {
                    let label = tunnel
                        .label
                        .clone()
                        .unwrap_or_else(|| format!(":{}", tunnel.local_port));
                    self.pending_delete = Some(PendingDelete::Tunnel {
                        id: tunnel.id,
                        label,
                    });
                    self.mode = AppMode::ConfirmDelete;
                }
            }
            _ if self.is_action(KeyAction::ToggleTunnel, &key) => self.toggle_tunnel()?,
            _ if self.is_action(KeyAction::TunnelKill, &key) => self.kill_selected_tunnel()?,
            KeyCode::Char('R') => {
                self.tunnel_reconnect_selected = 0;
                self.mode = AppMode::TunnelReconnectSettings;
            }
            _ if self.is_action(KeyAction::Help, &key) => {
                self.pre_help_mode = Some(self.mode);
                self.mode = AppMode::Help;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn toggle_tunnel(&mut self) -> Result<()> {
        let Some(tunnel) = self.tunnels.get(self.tunnel_selected).cloned() else {
            return Ok(());
        };
        let host = tunnel
            .host_id
            .and_then(|hid| self.store.get_host(hid).ok().flatten());
        let host_name = host.as_ref().map(|h| h.name.as_str()).unwrap_or("unknown");
        let label = tunnel.label.as_deref().unwrap_or("");

        if self.tunnel_manager.is_running(tunnel.id)
            || self.tunnel_manager.has_child(tunnel.id)
            || self.tunnel_manager.is_reconnecting(tunnel.id)
        {
            self.tunnel_manager.stop_user(tunnel.id)?;
            self.tunnel_notice = Some(format!("Stopped tunnel :{}", tunnel.local_port));
            let _ = self.store.log_auth_event(
                host_name,
                None,
                "tunnel",
                "ok",
                &format!("tunnel stopped :{} {}", tunnel.local_port, label),
                None,
            );
        } else {
            self.tunnel_manager.resume_auto_reconnect(tunnel.id);
            let (secret, _) = host
                .as_ref()
                .map(|h| resolve_pending_secret_for_managed(h, self.password_store.as_ref()))
                .unwrap_or((None, String::new()));
            match self
                .tunnel_manager
                .start(&tunnel, host.as_ref(), secret.as_ref())
            {
                Ok(()) => {
                    self.tunnel_notice = Some(format!("Started tunnel :{}", tunnel.local_port));
                    let _ = self.store.log_auth_event(
                        host_name,
                        None,
                        "tunnel",
                        "launched",
                        &format!("tunnel started :{} {}", tunnel.local_port, label),
                        None,
                    );
                }
                Err(e) => {
                    self.tunnel_notice = Some(format!("Failed: {e:#}"));
                    let _ = self.store.log_auth_event(
                        host_name,
                        None,
                        "tunnel",
                        "fail",
                        &format!("tunnel failed :{} — {e:#}", tunnel.local_port),
                        None,
                    );
                }
            }
        }
        Ok(())
    }

    pub(crate) fn kill_selected_tunnel(&mut self) -> Result<()> {
        let Some(tunnel) = self.tunnels.get(self.tunnel_selected).cloned() else {
            return Ok(());
        };
        let host_name = tunnel
            .host_id
            .and_then(|hid| self.store.get_host(hid).ok().flatten())
            .map(|h| h.name)
            .unwrap_or_else(|| "unknown".into());
        if self.tunnel_manager.is_running(tunnel.id)
            || self.tunnel_manager.is_reconnecting(tunnel.id)
        {
            self.tunnel_manager.stop_user(tunnel.id)?;
            self.tunnel_notice = Some(format!("Killed tunnel :{}", tunnel.local_port));
            let _ = self.store.log_auth_event(
                &host_name,
                None,
                "tunnel",
                "ok",
                &format!("tunnel killed :{}", tunnel.local_port),
                None,
            );
        }
        Ok(())
    }

    pub fn reload_tunnels(&mut self) -> Result<()> {
        self.tunnels = self.store.list_tunnels()?;
        if self.tunnel_selected >= self.tunnels.len() && !self.tunnels.is_empty() {
            self.tunnel_selected = self.tunnels.len() - 1;
        }
        Ok(())
    }

    pub(crate) fn handle_key_tunnel_form(&mut self, key: KeyEvent) -> Result<()> {
        let Some(form) = self.tunnel_form.as_ref() else {
            return Ok(());
        };
        let field = form.active_field;
        match key.code {
            KeyCode::Esc => {
                if self.tunnel_form.as_ref().is_some_and(|f| f.dirty) {
                    self.mode = AppMode::ConfirmDiscard;
                } else {
                    self.tunnel_form = None;
                    self.mode = AppMode::Normal;
                }
            }
            _ if self.is_save_key(&key) => self.save_tunnel_form()?,
            // The SSH server field opens a searchable picker instead of the
            // old one-at-a-time ←/→ cycle.
            KeyCode::Enter | KeyCode::Char(' ') if field == TunnelFormField::Host => {
                self.open_tunnel_host_picker();
            }
            KeyCode::Char(' ') if field == TunnelFormField::AutoConnect => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    form.auto_connect = !form.auto_connect;
                    form.dirty = true;
                }
            }
            // Single-step model: Enter on the last field saves.
            KeyCode::Enter if field == TunnelFormField::AutoConnect => self.save_tunnel_form()?,
            KeyCode::Enter if field == TunnelFormField::Label => self.save_tunnel_form()?,
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down if key.modifiers.is_empty() => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    form.active_field = form.active_field.next();
                    form.cursor = form
                        .active_text_field()
                        .map(text_input::char_len)
                        .unwrap_or(0);
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    form.active_field = form.active_field.prev();
                    form.cursor = form
                        .active_text_field()
                        .map(text_input::char_len)
                        .unwrap_or(0);
                }
            }
            KeyCode::Left | KeyCode::Right => {
                let mut open_picker = false;
                if let Some(form) = self.tunnel_form.as_mut() {
                    match form.active_field {
                        TunnelFormField::Type => {
                            form.tunnel_type = form.tunnel_type.next();
                            form.dirty = true;
                        }
                        TunnelFormField::AutoConnect => {
                            form.auto_connect = !form.auto_connect;
                            form.dirty = true;
                        }
                        // The server field is chosen via the searchable picker.
                        TunnelFormField::Host => open_picker = true,
                        // Text fields: move the edit cursor.
                        _ => {
                            let mut cursor = form.cursor;
                            if let Some(v) = form.active_text_field_mut() {
                                text_input::handle_cursor_key(key.code, v, &mut cursor);
                                form.cursor = cursor;
                            }
                        }
                    }
                }
                if open_picker {
                    self.open_tunnel_host_picker();
                }
            }
            KeyCode::Home | KeyCode::End | KeyCode::Delete => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    let mut cursor = form.cursor;
                    let changed = form
                        .active_text_field_mut()
                        .and_then(|v| text_input::handle_cursor_key(key.code, v, &mut cursor));
                    form.cursor = cursor;
                    if changed == Some(true) {
                        form.dirty = true;
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    let cursor = form.cursor;
                    let nc = match form.active_text_field_mut() {
                        Some(v) => text_input::backspace_at(v, cursor),
                        None => cursor,
                    };
                    if nc != cursor {
                        form.cursor = nc;
                        form.dirty = true;
                    }
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(form) = self.tunnel_form.as_mut() {
                    let cursor = form.cursor;
                    let nc = form
                        .active_text_field_mut()
                        .map(|v| text_input::insert_at(v, cursor, c));
                    if let Some(nc) = nc {
                        form.cursor = nc;
                        form.dirty = true;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Managed hosts matching the tunnel picker's current query, in list order,
    /// as `(id, display name)` pairs. All hosts when the query is empty.
    pub fn tunnel_host_matches(&self) -> Vec<(i64, String)> {
        let query = self
            .tunnel_host_picker
            .as_ref()
            .map(|p| p.query.to_lowercase())
            .unwrap_or_default();
        self.hosts
            .iter()
            .filter_map(|h| {
                let id = h.managed_id()?;
                let name = h.display_name().to_string();
                if query.is_empty() || name.to_lowercase().contains(&query) {
                    Some((id, name))
                } else {
                    None
                }
            })
            .collect()
    }

    pub(crate) fn open_tunnel_host_picker(&mut self) {
        if self.tunnel_form.is_none() {
            return;
        }
        // Preselect the currently chosen server, if any.
        let current = self.tunnel_form.as_ref().and_then(|f| f.host_id);
        let selected = current
            .and_then(|id| {
                self.hosts
                    .iter()
                    .filter_map(|h| h.managed_id())
                    .position(|h| h == id)
            })
            .unwrap_or(0);
        self.tunnel_host_picker = Some(TunnelHostPicker {
            query: String::new(),
            selected,
        });
        self.mode = AppMode::TunnelHostPicker;
    }

    pub(crate) fn handle_key_tunnel_host_picker(&mut self, key: KeyEvent) -> Result<()> {
        let len = self.tunnel_host_matches().len();
        match key.code {
            KeyCode::Esc => {
                self.tunnel_host_picker = None;
                self.mode = AppMode::TunnelForm;
            }
            KeyCode::Down => {
                if len > 0 {
                    if let Some(p) = self.tunnel_host_picker.as_mut() {
                        p.selected = (p.selected + 1) % len;
                    }
                }
            }
            KeyCode::Up => {
                if len > 0 {
                    if let Some(p) = self.tunnel_host_picker.as_mut() {
                        p.selected = (p.selected + len - 1) % len;
                    }
                }
            }
            KeyCode::Enter => {
                let matches = self.tunnel_host_matches();
                let chosen = self
                    .tunnel_host_picker
                    .as_ref()
                    .and_then(|p| matches.get(p.selected))
                    .map(|(id, _)| *id);
                if let (Some(id), Some(form)) = (chosen, self.tunnel_form.as_mut()) {
                    form.host_id = Some(id);
                    form.dirty = true;
                }
                self.tunnel_host_picker = None;
                self.mode = AppMode::TunnelForm;
            }
            KeyCode::Backspace => {
                if let Some(p) = self.tunnel_host_picker.as_mut() {
                    p.query.pop();
                    p.selected = 0;
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(p) = self.tunnel_host_picker.as_mut() {
                    p.query.push(c);
                    p.selected = 0;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn save_tunnel_form(&mut self) -> Result<()> {
        let Some(form) = self.tunnel_form.take() else {
            return Ok(());
        };

        let local_port: u16 = form.local_port.parse().unwrap_or(0);
        if local_port == 0 {
            self.tunnel_notice = Some("Invalid local port".into());
            self.tunnel_form = Some(form);
            return Ok(());
        }
        let remote_port: u16 = if form.tunnel_type == crate::store::TunnelType::Dynamic {
            0
        } else {
            form.remote_port.parse().unwrap_or(0)
        };

        let new = crate::store::NewTunnel {
            host_id: form.host_id,
            tunnel_type: form.tunnel_type,
            local_port,
            remote_host: form.remote_host,
            remote_port,
            label: if form.label.is_empty() {
                None
            } else {
                Some(form.label)
            },
            // Preserved below when editing an existing tunnel.
            auto_connect: form.auto_connect,
        };

        match form.editing_id {
            None => {
                self.store.create_tunnel(&new)?;
                self.tunnel_notice = Some(format!("Created tunnel :{local_port}"));
            }
            Some(id) => {
                // Editing recreates the row under a fresh id, so a still-running
                // child would be orphaned (holding its local port, invisible to
                // the UI). Stop it first, mirroring the delete path.
                if self.tunnel_manager.is_running(id)
                    || self.tunnel_manager.is_reconnecting(id)
                    || self.tunnel_manager.is_gave_up(id)
                {
                    self.tunnel_manager.stop_user(id)?;
                }
                self.store.delete_tunnel(id)?;
                self.store.create_tunnel(&new)?;
                self.tunnel_notice = Some(format!("Updated tunnel :{local_port}"));
            }
        }

        self.reload_tunnels()?;
        self.mode = AppMode::Normal;
        Ok(())
    }

    pub(crate) fn bootstrap_auto_connect_tunnels(&mut self) -> Result<()> {
        self.reload_tunnels()?;
        let tunnels: Vec<_> = self
            .tunnels
            .iter()
            .filter(|t| t.auto_connect)
            .cloned()
            .collect();
        for tunnel in tunnels {
            if self.tunnel_manager.is_running(tunnel.id)
                || self.tunnel_manager.is_reconnecting(tunnel.id)
            {
                continue;
            }
            let host = tunnel
                .host_id
                .and_then(|hid| self.store.get_host(hid).ok().flatten());
            let host_name = host.as_ref().map(|h| h.name.as_str()).unwrap_or("unknown");
            let label = tunnel.label.as_deref().unwrap_or("");
            self.tunnel_manager.resume_auto_reconnect(tunnel.id);
            let (secret, _) = host
                .as_ref()
                .map(|h| resolve_pending_secret_for_managed(h, self.password_store.as_ref()))
                .unwrap_or((None, String::new()));
            match self
                .tunnel_manager
                .start(&tunnel, host.as_ref(), secret.as_ref())
            {
                Ok(()) => {
                    let _ = self.store.log_auth_event(
                        host_name,
                        None,
                        "tunnel",
                        "launched",
                        &format!("tunnel started (auto) :{} {}", tunnel.local_port, label),
                        None,
                    );
                }
                Err(e) => {
                    let err = format!("{e:#}");
                    self.tunnel_manager.on_auto_start_failed(
                        tunnel.id,
                        &err,
                        &self.config.tunnel_reconnect,
                    );
                    let _ = self.store.log_auth_event(
                        host_name,
                        None,
                        "tunnel",
                        "fail",
                        &format!(
                            "tunnel auto-start failed :{} {} — {e:#}",
                            tunnel.local_port, label
                        ),
                        None,
                    );
                }
            }
        }
        Ok(())
    }

    pub(crate) fn log_tunnel_reconnect_events(
        &self,
        events: &[crate::tunnel::ReconnectEvent],
        tunnels: &[crate::store::Tunnel],
    ) {
        for ev in events {
            let tunnel = tunnels.iter().find(|t| t.id == ev.tunnel_id());
            let (host_name, port, label) = tunnel
                .map(|t| {
                    let name = t
                        .host_id
                        .and_then(|hid| self.store.get_host(hid).ok().flatten())
                        .map(|h| h.name)
                        .unwrap_or_else(|| "unknown".into());
                    (name, t.local_port, t.label.clone().unwrap_or_default())
                })
                .unwrap_or_else(|| ("unknown".into(), 0, String::new()));

            match ev {
                crate::tunnel::ReconnectEvent::Attempt { attempt, .. } => {
                    let _ = self.store.log_auth_event(
                        &host_name,
                        None,
                        "tunnel",
                        "retry",
                        &format!("tunnel reconnecting :{} {} attempt {attempt}", port, label),
                        None,
                    );
                }
                crate::tunnel::ReconnectEvent::Reconnected { .. } => {
                    let _ = self.store.log_auth_event(
                        &host_name,
                        None,
                        "tunnel",
                        "launched",
                        &format!("tunnel reconnected :{} {}", port, label),
                        None,
                    );
                }
                crate::tunnel::ReconnectEvent::GaveUp {
                    attempts, error, ..
                } => {
                    let _ = self.store.log_auth_event(
                        &host_name,
                        None,
                        "tunnel",
                        "fail",
                        &format!(
                            "tunnel gave up :{} {} after {attempts} attempts — {error}",
                            port, label
                        ),
                        None,
                    );
                }
            }
        }
    }

    pub(crate) fn tick_tunnel_reconnect(&mut self) -> Result<()> {
        if self.should_quit {
            return Ok(());
        }
        let cfg = self.config.tunnel_reconnect.clone();
        let tunnels = self.tunnels.clone();
        let store = Arc::clone(&self.store);
        let events = self.tunnel_manager.tick_reconnect(
            &tunnels,
            &cfg,
            |host_id| store.get_host(host_id).ok().flatten(),
            |host| {
                resolve_pending_secret_for_managed(host, self.password_store.as_ref()).0
            },
        );
        self.log_tunnel_reconnect_events(&events, &tunnels);
        Ok(())
    }

    pub(crate) fn tick_tunnels(&mut self) -> Result<()> {
        if !self.tunnels_auto_started {
            self.bootstrap_auto_connect_tunnels()?;
            self.tunnels_auto_started = true;
        }
        if self.tunnel_manager.needs_tunnel_list() && self.tunnels.is_empty() {
            self.reload_tunnels()?;
        }
        let health_events = self.tunnel_manager.check_health(
            &self.tunnels,
            &self.config.tunnel_reconnect,
        );
        self.log_tunnel_reconnect_events(&health_events, &self.tunnels);
        self.tick_tunnel_reconnect()
    }

    pub(crate) fn handle_key_tunnel_reconnect_settings(&mut self, key: KeyEvent) -> Result<()> {
        use crate::app::TUNNEL_RECONNECT_FIELDS;
        let n = TUNNEL_RECONNECT_FIELDS.len();
        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => self.mode = AppMode::Normal,
            _ if self.is_action(KeyAction::MoveDown, &key) => {
                self.tunnel_reconnect_selected = (self.tunnel_reconnect_selected + 1) % n;
            }
            _ if self.is_action(KeyAction::MoveUp, &key) => {
                self.tunnel_reconnect_selected =
                    (self.tunnel_reconnect_selected + n - 1) % n;
            }
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Right => {
                self.config.tunnel_reconnect.adjust_field(self.tunnel_reconnect_selected, 1);
                self.save_config_quietly();
            }
            KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Left => {
                self.config.tunnel_reconnect.adjust_field(self.tunnel_reconnect_selected, -1);
                self.save_config_quietly();
            }
            KeyCode::Char('*') => {
                self.config
                    .tunnel_reconnect
                    .reset_field(self.tunnel_reconnect_selected);
                self.save_config_quietly();
            }
            _ => {}
        }
        Ok(())
    }
}
