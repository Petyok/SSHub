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
                    active_field: TunnelFormField::Host,
                    editing: true,
                    edit_snapshot: String::new(),
                    dirty: false,
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
                        active_field: TunnelFormField::Host,
                        editing: true,
                        edit_snapshot: String::new(),
                        dirty: false,
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

        if self.tunnel_manager.is_running(tunnel.id) {
            self.tunnel_manager.stop(tunnel.id)?;
            self.tunnel_notice = Some(format!("Stopped tunnel :{}", tunnel.local_port));
            let _ = self.store.log_auth_event(
                host_name,
                None,
                "tunnel",
                "ok",
                &format!("tunnel stopped :{} {}", tunnel.local_port, label),
            );
        } else {
            match self.tunnel_manager.start(&tunnel, host.as_ref()) {
                Ok(()) => {
                    self.tunnel_notice = Some(format!("Started tunnel :{}", tunnel.local_port));
                    let _ = self.store.log_auth_event(
                        host_name,
                        None,
                        "tunnel",
                        "launched",
                        &format!("tunnel started :{} {}", tunnel.local_port, label),
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
                    );
                }
            }
        }
        Ok(())
    }

    pub(crate) fn kill_selected_tunnel(&mut self) -> Result<()> {
        let Some(tunnel) = self.tunnels.get(self.tunnel_selected) else {
            return Ok(());
        };
        if self.tunnel_manager.is_running(tunnel.id) {
            let host_name = tunnel
                .host_id
                .and_then(|hid| self.store.get_host(hid).ok().flatten())
                .map(|h| h.name)
                .unwrap_or_else(|| "unknown".into());
            self.tunnel_manager.stop(tunnel.id)?;
            self.tunnel_notice = Some(format!("Killed tunnel :{}", tunnel.local_port));
            let _ = self.store.log_auth_event(
                &host_name,
                None,
                "tunnel",
                "ok",
                &format!("tunnel killed :{}", tunnel.local_port),
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
            // Single-step model: Enter on the last field saves.
            KeyCode::Enter if field == TunnelFormField::Label => self.save_tunnel_form()?,
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down if key.modifiers.is_empty() => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    form.active_field = form.active_field.next();
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    form.active_field = form.active_field.prev();
                }
            }
            KeyCode::Left | KeyCode::Right => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    if form.active_field == TunnelFormField::Type {
                        form.tunnel_type = form.tunnel_type.next();
                        form.dirty = true;
                    } else if form.active_field == TunnelFormField::Host {
                        // The server field is chosen via the searchable picker.
                        self.open_tunnel_host_picker();
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    let field = match form.active_field {
                        TunnelFormField::LocalPort => &mut form.local_port,
                        TunnelFormField::RemoteHost => &mut form.remote_host,
                        TunnelFormField::RemotePort => &mut form.remote_port,
                        TunnelFormField::Label => &mut form.label,
                        _ => return Ok(()),
                    };
                    if field.pop().is_some() {
                        form.dirty = true;
                    }
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(form) = self.tunnel_form.as_mut() {
                    let field = match form.active_field {
                        TunnelFormField::LocalPort => &mut form.local_port,
                        TunnelFormField::RemoteHost => &mut form.remote_host,
                        TunnelFormField::RemotePort => &mut form.remote_port,
                        TunnelFormField::Label => &mut form.label,
                        _ => return Ok(()),
                    };
                    field.push(c);
                    form.dirty = true;
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
            auto_connect: false,
        };

        match form.editing_id {
            None => {
                self.store.create_tunnel(&new)?;
                self.tunnel_notice = Some(format!("Created tunnel :{local_port}"));
            }
            Some(id) => {
                // Recreate, carrying over fields the form doesn't expose.
                let mut new = new;
                if let Some(existing) = self.tunnels.iter().find(|t| t.id == id) {
                    new.auto_connect = existing.auto_connect;
                }
                // Editing recreates the row under a fresh id, so a still-running
                // child would be orphaned (holding its local port, invisible to
                // the UI). Stop it first, mirroring the delete path.
                if self.tunnel_manager.is_running(id) {
                    self.tunnel_manager.stop(id)?;
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
}
