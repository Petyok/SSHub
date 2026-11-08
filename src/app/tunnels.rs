use super::*;

impl App {
    pub(crate) fn switch_to_tunnels_tab(&mut self) -> Result<()> {
        self.active_tab = 2;
        self.reload_tunnels()?;
        Ok(())
    }

    /// `tunnels` indices the Tunnels tab shows: all of them, or those whose
    /// label, host, ports or type fuzzy-match the filter, best first.
    pub fn tunnel_view(&self) -> Vec<usize> {
        if self.tunnel_filter.is_empty() {
            return (0..self.tunnels.len()).collect();
        }
        let items: Vec<Vec<String>> = self
            .tunnels
            .iter()
            .map(|t| {
                let host = t
                    .host_id
                    .and_then(|hid| self.store.get_host(hid).ok().flatten())
                    .map(|h| h.name);
                vec![
                    t.label.clone().unwrap_or_default(),
                    host.unwrap_or_default(),
                    t.remote_host.clone(),
                    t.local_port.to_string(),
                    t.remote_port.to_string(),
                    t.tunnel_type.as_str().to_string(),
                ]
            })
            .collect();
        crate::search::fuzzy_indices(&self.tunnel_filter, &items)
    }

    /// Keep `tunnel_selected` on a visible row after the filter or list changed.
    pub(crate) fn snap_tunnel_selection(&mut self) {
        let view = self.tunnel_view();
        if !view.contains(&self.tunnel_selected) {
            self.tunnel_selected = view.first().copied().unwrap_or(0);
        }
    }

    /// The selected tunnel, or `None` when the filter hides it (or hides
    /// every row), so no action can reach a row the user cannot see.
    pub(crate) fn selected_tunnel(&self) -> Option<&crate::store::Tunnel> {
        self.tunnel_view()
            .contains(&self.tunnel_selected)
            .then(|| self.tunnels.get(self.tunnel_selected))
            .flatten()
    }

    /// Step the selection through the visible rows.
    pub(crate) fn move_tunnel_selection(&mut self, delta: i32) {
        let view = self.tunnel_view();
        if view.is_empty() {
            return;
        }
        let pos = view
            .iter()
            .position(|&i| i == self.tunnel_selected)
            .unwrap_or(0) as i32;
        let next = (pos + delta).clamp(0, view.len() as i32 - 1);
        self.tunnel_selected = view[next as usize];
    }

    /// Keystrokes while the filter bar has focus. Enter keeps the filter, Esc
    /// drops it.
    fn handle_key_tunnel_filter(&mut self, key: KeyEvent) {
        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => {
                self.tunnel_filter.clear();
                self.list_filter_editing = false;
            }
            _ if self.is_action(KeyAction::Connect, &key) => self.list_filter_editing = false,
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.tunnel_filter.push(c);
            }
            KeyCode::Backspace => {
                self.tunnel_filter.pop();
            }
            KeyCode::Up => self.move_tunnel_selection(-1),
            KeyCode::Down => self.move_tunnel_selection(1),
            _ => {}
        }
        self.snap_tunnel_selection();
    }

    pub(crate) fn handle_key_tunnels(&mut self, key: KeyEvent) -> Result<()> {
        self.tunnel_notice = None;

        if self.list_filter_editing {
            self.handle_key_tunnel_filter(key);
            return Ok(());
        }

        if self.try_tab_switch(&key)? {
            return Ok(());
        }

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::Search, &key) => self.list_filter_editing = true,
            _ if self.is_action(KeyAction::Cancel, &key) && !self.tunnel_filter.is_empty() => {
                self.tunnel_filter.clear();
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_tunnel_selection(1),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_tunnel_selection(-1),
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
                if let Some(tunnel) = self.selected_tunnel().cloned() {
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
                if let Some(tunnel) = self.selected_tunnel().cloned() {
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
                self.open_help();
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn toggle_tunnel(&mut self) -> Result<()> {
        let Some(id) = self.selected_tunnel().map(|t| t.id) else {
            return Ok(());
        };
        self.toggle_tunnel_by_id(id)
    }

    /// Start or stop tunnel `id`, whichever tab asked: the tunnels tab passes
    /// its selection, the network map the tunnel of the host under its cursor.
    pub(crate) fn toggle_tunnel_by_id(&mut self, id: i64) -> Result<()> {
        let Some(tunnel) = self.tunnels.iter().find(|t| t.id == id).cloned() else {
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
            // Effective connection (host-explicit → group chain → global), the
            // same resolution every connect path uses; a failure falls back to
            // the stored row rather than refusing the tunnel.
            let start_result = match host.as_ref() {
                Some(h) => {
                    let resolved = self
                        .store
                        .resolve_connection(h)
                        .unwrap_or_else(|_| crate::store::ResolvedConnection::from_stored(h));
                    let hops = hop_secrets(
                        &self.store,
                        &self.hosts,
                        self.password_store.as_ref(),
                        resolved.proxy_jump.as_deref(),
                    );
                    self.tunnel_manager
                        .start(&tunnel, h, &resolved, secret.as_ref(), &hops)
                }
                None => Err(anyhow::anyhow!("No host associated with tunnel")),
            };
            match start_result {
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
                    if tunnel.auto_connect {
                        let err = format!("{e:#}");
                        let gave_up = self.tunnel_manager.on_auto_start_failed(
                            tunnel.id,
                            &err,
                            &self.config.tunnel_reconnect,
                        );
                        if let Some(ev) = gave_up {
                            crate::tunnel::log_tunnel_reconnect_events(
                                &self.store,
                                std::slice::from_ref(&ev),
                                std::slice::from_ref(&tunnel),
                            );
                        } else {
                            let _ = self.store.log_auth_event(
                                host_name,
                                None,
                                "tunnel",
                                "fail",
                                &format!(
                                    "tunnel start failed :{} {} — {e:#}",
                                    tunnel.local_port, label
                                ),
                                None,
                            );
                        }
                    } else {
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
        }
        Ok(())
    }

    pub(crate) fn kill_selected_tunnel(&mut self) -> Result<()> {
        let Some(tunnel) = self.selected_tunnel().cloned() else {
            return Ok(());
        };
        let host_name = tunnel
            .host_id
            .and_then(|hid| self.store.get_host(hid).ok().flatten())
            .map(|h| h.name)
            .unwrap_or_else(|| "unknown".into());
        if self.tunnel_manager.is_running(tunnel.id)
            || self.tunnel_manager.has_child(tunnel.id)
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
        self.snap_tunnel_selection();
        Ok(())
    }

    /// Ctrl+U on the tunnel form: kill the active text field back to its start.
    pub(crate) fn tunnel_form_kill_to_start(&mut self) {
        let Some(form) = self.tunnel_form.as_mut() else {
            return;
        };
        if form.cursor > 0 {
            let cursor = form.cursor;
            let nc = form
                .active_text_field_mut()
                .map(|v| text_input::clear_before_cursor(v, cursor))
                .unwrap_or(cursor);
            form.cursor = nc;
            form.dirty = true;
        }
    }

    /// Ctrl+W on the tunnel form: delete the word before the cursor.
    pub(crate) fn tunnel_form_kill_word(&mut self) {
        let Some(form) = self.tunnel_form.as_mut() else {
            return;
        };
        let len_before = form.active_text_field().map(text_input::char_len);
        let cursor = form.cursor;
        let nc = form
            .active_text_field_mut()
            .map(|v| text_input::delete_word_before(v, cursor))
            .unwrap_or(cursor);
        form.cursor = nc;
        if len_before.is_some_and(|before| {
            form.active_text_field().map(text_input::char_len) != Some(before)
        }) {
            form.dirty = true;
        }
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
            // Readline-style bulk edits: Ctrl+U clears the field (a wrong port
            // need not be backspaced digit by digit), Ctrl+W eats one word.
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                self.tunnel_form_kill_to_start()
            }
            KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => {
                self.tunnel_form_kill_word()
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
        let Some(mut form) = self.tunnel_form.take() else {
            return Ok(());
        };

        let local_port: u16 = form.local_port.trim().parse().unwrap_or(0);
        let remote_text = form.remote_port.trim();
        let remote_port: Option<u16> = if form.tunnel_type == crate::store::TunnelType::Dynamic {
            Some(0)
        } else if remote_text.is_empty() {
            // Forwarding to the same port number is the common case.
            Some(local_port)
        } else {
            remote_text.parse().ok().filter(|&p| p != 0)
        };
        let error = if form.host_id.is_none() {
            Some((TunnelFormField::Host, "Pick an SSH server"))
        } else if local_port == 0 {
            Some((TunnelFormField::LocalPort, "Invalid local port"))
        } else if remote_port.is_none() {
            Some((TunnelFormField::RemotePort, "Invalid destination port"))
        } else {
            None
        };
        if let Some((field, msg)) = error {
            // Drawn on the form's own error row; focus jumps to the bad field.
            self.tunnel_notice = Some(msg.into());
            form.active_field = field;
            form.cursor = form
                .active_text_field()
                .map(text_input::char_len)
                .unwrap_or(0);
            self.tunnel_form = Some(form);
            return Ok(());
        }
        let remote_port = remote_port.unwrap_or(0);

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

        let mut restart = None;
        match form.editing_id {
            None => {
                self.store.create_tunnel(&new)?;
                self.tunnel_notice = Some(format!("Created tunnel :{local_port}"));
            }
            Some(id) => {
                // Editing recreates the row under a fresh id, so a still-running
                // child would be orphaned (holding its local port, invisible to
                // the UI). Stop it first, mirroring the delete path, and bring
                // the new row back up below if it was live.
                let live = self.tunnel_manager.is_running(id)
                    || self.tunnel_manager.has_child(id)
                    || self.tunnel_manager.is_reconnecting(id);
                if live || self.tunnel_manager.is_gave_up(id) {
                    self.tunnel_manager.stop_user(id)?;
                }
                self.store.delete_tunnel(id)?;
                let new_id = self.store.create_tunnel(&new)?;
                self.tunnel_notice = Some(format!("Updated tunnel :{local_port}"));
                restart = live.then_some(new_id);
            }
        }

        self.reload_tunnels()?;
        if let Some(new_id) = restart {
            self.toggle_tunnel_by_id(new_id)?;
            self.tunnel_notice = Some(if self.tunnel_manager.has_child(new_id) {
                format!("Updated and restarted tunnel :{local_port}")
            } else {
                // toggle_tunnel_by_id left the start failure in the notice.
                let why = self.tunnel_notice.take().unwrap_or_default();
                format!("Updated tunnel :{local_port}; it stopped and did not restart ({why})")
            });
        }
        let hidden = !self.tunnel_filter.is_empty()
            && !self
                .tunnel_view()
                .iter()
                .any(|&i| self.tunnels[i].local_port == local_port);
        if hidden {
            if let Some(n) = self.tunnel_notice.as_mut() {
                n.push_str(" (hidden by the filter)");
            }
        }
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
                || self.tunnel_manager.has_child(tunnel.id)
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
            let start_result = match host.as_ref() {
                Some(h) => {
                    let resolved = self
                        .store
                        .resolve_connection(h)
                        .unwrap_or_else(|_| crate::store::ResolvedConnection::from_stored(h));
                    let hops = hop_secrets(
                        &self.store,
                        &self.hosts,
                        self.password_store.as_ref(),
                        resolved.proxy_jump.as_deref(),
                    );
                    self.tunnel_manager
                        .start(&tunnel, h, &resolved, secret.as_ref(), &hops)
                }
                None => Err(anyhow::anyhow!("No host associated with tunnel")),
            };
            match start_result {
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
                    let gave_up = self.tunnel_manager.on_auto_start_failed(
                        tunnel.id,
                        &err,
                        &self.config.tunnel_reconnect,
                    );
                    if let Some(ev) = gave_up {
                        crate::tunnel::log_tunnel_reconnect_events(
                            &self.store,
                            std::slice::from_ref(&ev),
                            std::slice::from_ref(&tunnel),
                        );
                    } else {
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
        }
        Ok(())
    }

    pub(crate) fn tick_tunnel_reconnect(&mut self) -> Result<()> {
        if self.should_quit {
            return Ok(());
        }
        let store = Arc::clone(&self.store);
        let events = self.tunnel_manager.tick_reconnect(
            &self.tunnels,
            &self.config.tunnel_reconnect,
            |host_id| {
                store.get_host(host_id).ok().flatten().map(|h| {
                    let resolved = store
                        .resolve_connection(&h)
                        .unwrap_or_else(|_| crate::store::ResolvedConnection::from_stored(&h));
                    (h, resolved)
                })
            },
            |host, resolved| {
                (
                    resolve_pending_secret_for_managed(host, self.password_store.as_ref()).0,
                    hop_secrets(
                        &store,
                        &self.hosts,
                        self.password_store.as_ref(),
                        resolved.proxy_jump.as_deref(),
                    ),
                )
            },
        );
        crate::tunnel::log_tunnel_reconnect_events(&self.store, &events, &self.tunnels);
        Ok(())
    }

    /// Re-read the tunnel list from the store, throttled to once every two
    /// seconds.
    ///
    /// The list is a snapshot, and the store is shared: a second sshub window or
    /// a `sshub tunnel` command can add, edit or delete a row under us. Until
    /// this ran, a tunnel deleted in another window stayed in this window's
    /// session top bar and tunnels tab for as long as the window lived.
    ///
    /// The selection is re-anchored by id, the way [`Self::reload_identities`]
    /// does: it is a raw index, and another window deleting a row above it used
    /// to slide it onto a different tunnel — with the reload now running on a
    /// timer, the next `d` would have hit whatever moved under the cursor.
    ///
    /// Our own children are left alone. An edit elsewhere is a delete plus an
    /// insert (there is no `UPDATE` for tunnels), so "this id is gone" does not
    /// mean "this tunnel is gone", and killing the child on that signal would
    /// take down a tunnel someone merely relabelled in another window.
    pub(crate) fn sync_tunnels_from_store(&mut self) -> Result<()> {
        self.tunnels_synced = std::time::Instant::now();
        let selected_id = self.tunnels.get(self.tunnel_selected).map(|t| t.id);
        self.reload_tunnels()?;
        self.tunnel_selected = selected_id
            .and_then(|id| self.tunnels.iter().position(|t| t.id == id))
            .unwrap_or(self.tunnel_selected)
            .min(self.tunnels.len().saturating_sub(1));
        self.snap_tunnel_selection();
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
        if self.tunnels_synced.elapsed() >= std::time::Duration::from_secs(2) {
            self.sync_tunnels_from_store()?;
        }
        let health_events = self
            .tunnel_manager
            .check_health(&self.tunnels, &self.config.tunnel_reconnect);
        crate::tunnel::log_tunnel_reconnect_events(&self.store, &health_events, &self.tunnels);
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
                self.tunnel_reconnect_selected = (self.tunnel_reconnect_selected + n - 1) % n;
            }
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Right => {
                self.config
                    .tunnel_reconnect
                    .adjust_field(self.tunnel_reconnect_selected, 1);
                self.save_config_quietly();
            }
            KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Left => {
                self.config
                    .tunnel_reconnect
                    .adjust_field(self.tunnel_reconnect_selected, -1);
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
