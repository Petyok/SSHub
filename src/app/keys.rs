use super::*;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.mode == AppMode::SessionHostPicker {
            return self.handle_key_session_host_picker(key);
        }

        // When an embedded session is active, Ctrl+C inside the terminal must
        // reach the remote shell — not quit sshub. Session mode intercepts all
        // keys (except detach / tab keys) before this check.
        if matches!(self.mode, AppMode::Connecting | AppMode::Session) {
            return self.handle_key_session(key);
        }

        if self.is_action(KeyAction::ForceQuit, &key) {
            // First Ctrl+C asks for confirmation (if enabled); a second Ctrl+C
            // while the dialog is up forces the quit.
            if self.mode == AppMode::ConfirmQuit || !self.config.appearance.confirm_quit {
                self.should_quit = true;
            } else {
                self.pre_quit_mode = Some(self.mode);
                self.mode = AppMode::ConfirmQuit;
            }
            return Ok(());
        }

        // Keybinding editor from the dashboard navigation screens.
        if self.mode == AppMode::Normal && self.is_action(KeyAction::KeybindEditor, &key) {
            self.keybind_editor = Some(KeybindEditor {
                selected: 0,
                scroll: 0,
                capturing: false,
                append: false,
            });
            self.mode = AppMode::KeybindEditor;
            return Ok(());
        }

        // Settings overlay (Ctrl+H) from the dashboard.
        if self.mode == AppMode::Normal
            && key.code == KeyCode::Char('h')
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.settings_selected = 0;
            self.mode = AppMode::Settings;
            return Ok(());
        }

        match self.mode {
            AppMode::KeybindEditor => self.handle_key_keybind_editor(key),
            AppMode::Settings => self.handle_key_settings(key),
            AppMode::TunnelReconnectSettings => self.handle_key_tunnel_reconnect_settings(key),
            AppMode::ConfirmQuit => self.handle_key_confirm_quit(key),
            AppMode::Help => self.handle_key_help(key),
            AppMode::ConfirmDiscard => self.handle_key_confirm_discard(key),
            AppMode::ConfirmDelete => self.handle_key_confirm_delete(key),
            AppMode::HostForm => self.handle_key_host_form(key),
            AppMode::IdentityForm => self.handle_key_identity_form(key),
            AppMode::GroupForm => self.handle_key_group_form(key),
            AppMode::GroupFieldPicker => self.handle_key_group_field_picker(key),
            AppMode::TunnelHostPicker => self.handle_key_tunnel_host_picker(key),
            AppMode::SessionHostPicker => self.handle_key_session_host_picker(key),
            AppMode::FieldPicker => self.handle_key_field_picker(key),
            AppMode::ImportPrompt => self.handle_key_import_prompt(key),
            AppMode::SftpPrompt => self.handle_key_sftp_prompt(key),
            AppMode::GroupManage => self.handle_key_group_manage(key),
            AppMode::Palette => self.handle_key_palette(key),
            AppMode::Search => self.handle_key_search(key),
            AppMode::TagFilter => self.handle_key_tag_filter(key),
            AppMode::HostDetail => self.handle_key_host_detail(key),
            AppMode::TunnelForm => self.handle_key_tunnel_form(key),
            AppMode::Connecting | AppMode::Session => self.handle_key_session(key),
            AppMode::Normal => match self.active_tab {
                1 => self.handle_key_sftp(key),
                2 => self.handle_key_tunnels(key),
                3 => self.handle_key_keychain(key),
                4 => self.handle_key_audit(key),
                _ => self.handle_key_normal(key),
            },
        }
    }

    pub(crate) fn handle_key_normal(&mut self, key: KeyEvent) -> Result<()> {
        self.host_notice = None;

        if self.handle_key_background_sessions(&key) {
            return Ok(());
        }

        if self.try_tab_switch(&key)? {
            return Ok(());
        }

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::MoveHostUp, &key) => self.move_host_manual(-1)?,
            _ if self.is_action(KeyAction::MoveHostDown, &key) => self.move_host_manual(1)?,
            _ if self.is_action(KeyAction::MoveGroupUp, &key) => self.move_selection_by_group(-1),
            _ if self.is_action(KeyAction::MoveGroupDown, &key) => self.move_selection_by_group(1),
            // Scroll the zoomed panel (except the hosts tree, which keeps its own
            // selection navigation). MUST precede the MoveDown/MoveUp arms below,
            // or those would shadow it and move the hidden host selection instead.
            _ if self.panel_zoomed
                && self.focused_panel != PanelId::Hosts
                && (self.is_action(KeyAction::MoveDown, &key) || key.code == KeyCode::PageDown) =>
            {
                let step = if key.code == KeyCode::PageDown { 10 } else { 1 };
                if self.focused_panel == PanelId::SshLog {
                    // The log scroll counts back from the latest line.
                    self.ssh_log_scroll = self.ssh_log_scroll.saturating_sub(step as usize);
                } else {
                    self.panel_scroll
                        .set(self.panel_scroll.get().saturating_add(step));
                }
            }
            _ if self.panel_zoomed
                && self.focused_panel != PanelId::Hosts
                && (self.is_action(KeyAction::MoveUp, &key) || key.code == KeyCode::PageUp) =>
            {
                let step = if key.code == KeyCode::PageUp { 10 } else { 1 };
                if self.focused_panel == PanelId::SshLog {
                    self.ssh_log_scroll = self.ssh_log_scroll.saturating_add(step as usize);
                } else {
                    self.panel_scroll
                        .set(self.panel_scroll.get().saturating_sub(step));
                }
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_selection(1),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_selection(-1),
            _ if self.is_action(KeyAction::Cancel, &key) && self.panel_zoomed => {
                self.panel_zoomed = false;
                self.panel_scroll.set(0);
            }
            _ if self.is_action(KeyAction::Cancel, &key) && !self.tag_filters.is_empty() => {
                self.tag_filters.clear();
                self.search_query.clear();
                self.rebuild_filter();
            }
            _ if self.is_action(KeyAction::ToggleGroup, &key) => self.toggle_selected_group(),
            _ if self.is_action(KeyAction::FoldGroupIn, &key) => {
                if self
                    .selected_nav_header()
                    .is_some_and(|si| !self.group_sections[si].collapsed)
                {
                    self.toggle_selected_group();
                }
            }
            _ if self.is_action(KeyAction::FoldGroupOut, &key) => {
                if self
                    .selected_nav_header()
                    .is_some_and(|si| self.group_sections[si].collapsed)
                {
                    self.toggle_selected_group();
                }
            }
            _ if self.is_action(KeyAction::CollapseAll, &key) => {
                let all_collapsed = !self.group_sections.is_empty()
                    && self.group_sections.iter().all(|s| s.collapsed);
                self.set_all_groups_collapsed(!all_collapsed);
            }
            _ if self.selected_nav_header().is_some()
                && self.is_action(KeyAction::Connect, &key) =>
            {
                self.toggle_selected_group()
            }
            _ if self.is_action(KeyAction::Connect, &key) => self.connect_selected()?,
            _ if self.is_action(KeyAction::AddHost, &key) => self.enter_host_form(None, false)?,
            _ if self.is_action(KeyAction::Delete, &key) => self.delete_selected_host()?,
            _ if self.is_action(KeyAction::Duplicate, &key) => self.duplicate_selected_host()?,
            _ if self.is_action(KeyAction::ExportSsh, &key) => match self.export_ssh_config() {
                Ok(path) => {
                    let count = self
                        .store
                        .list_hosts_filtered(Some(HostSource::Launcher))
                        .map(|h| h.len())
                        .unwrap_or(0);
                    self.host_notice =
                        Some(format!("Exported {count} host(s) to {}", path.display()));
                }
                Err(e) => self.host_notice = Some(format!("Export failed: {e:#}")),
            },
            _ if self.is_action(KeyAction::ImportSsh, &key) => match self.import_ssh_config() {
                Ok(report) => {
                    let mut msg = format!(
                        "Imported {} new, {} updated, {} skipped",
                        report.inserted, report.updated, report.skipped_launcher
                    );
                    if report.failed > 0 {
                        msg.push_str(&format!(", {} failed", report.failed));
                    }
                    self.host_notice = Some(msg);
                }
                Err(e) => self.host_notice = Some(format!("Import failed: {e:#}")),
            },
            _ if self.is_action(KeyAction::ImportTermius, &key) => self.open_import_prompt(),
            _ if self.is_action(KeyAction::Edit, &key) => {
                if self.selected_nav_header().is_some() {
                    // Edit the selected group (name, parent, default identity).
                    self.rename_selected_host_group()?;
                } else {
                    self.edit_selected_host()?;
                }
            }
            _ if self.is_action(KeyAction::UiZoomIn, &key) => {
                self.set_ui_zoom((self.ui_zoom + 1).min(UI_ZOOM_MAX));
            }
            _ if self.is_action(KeyAction::UiZoomOut, &key) => {
                self.set_ui_zoom(self.ui_zoom.saturating_sub(1));
            }
            _ if self.is_action(KeyAction::TogglePanelZoom, &key) => {
                self.panel_zoomed = !self.panel_zoomed;
                self.panel_scroll.set(0);
            }
            _ if self.is_action(KeyAction::FocusPanelLeft, &key) => {
                self.focus_panel(FocusDir::Left)
            }
            _ if self.is_action(KeyAction::FocusPanelRight, &key) => {
                self.focus_panel(FocusDir::Right)
            }
            _ if self.is_action(KeyAction::FocusPanelUp, &key) => self.focus_panel(FocusDir::Up),
            _ if self.is_action(KeyAction::FocusPanelDown, &key) => {
                self.focus_panel(FocusDir::Down)
            }
            _ if self.is_action(KeyAction::Favorite, &key) => self.toggle_favorite()?,
            _ if self.is_action(KeyAction::DetailFocus, &key) => {
                self.detail_focus = !self.detail_focus;
            }
            _ if self.is_action(KeyAction::Search, &key) => {
                self.palette_query.clear();
                self.palette_selected = 0;
                self.palette_results = (0..self.hosts.len()).collect();
                self.mode = AppMode::Palette;
            }
            _ if self.is_action(KeyAction::Help, &key) => {
                self.pre_help_mode = Some(self.mode);
                self.mode = AppMode::Help;
            }
            _ if self.is_action(KeyAction::TagFilter, &key) => self.open_tag_filter(),
            _ if self.is_action(KeyAction::ClearSshLog, &key) => {
                self.ssh_log.clear();
                self.ssh_log_scroll = 0;
                self.probe_rx = None;
                self.host_notice = Some("SSH log cleared.".into());
            }
            _ if self.is_action(KeyAction::SortCycle, &key) => self.cycle_sort_mode(),
            _ if self.is_action(KeyAction::YankLog, &key) => self.yank_ssh_log()?,
            _ if self.is_action(KeyAction::DeleteGroup, &key) => {
                self.delete_selected_host_group()?
            }
            _ if self.is_action(KeyAction::GroupsManage, &key) => self.enter_group_manage()?,
            _ if self.is_action(KeyAction::RenameGroup, &key) => {
                self.rename_selected_host_group()?
            }
            _ => {}
        }
        Ok(())
    }

    /// Move dashboard panel focus one step in `dir`; a no-op at a grid edge.
    fn focus_panel(&mut self, dir: FocusDir) {
        if let Some(next) = self.focused_panel.neighbor(dir) {
            self.focused_panel = next;
            self.panel_scroll.set(0);
        }
    }

    /// Switch dashboard tabs when a tab keybinding matches.
    pub(crate) fn try_tab_switch(&mut self, key: &KeyEvent) -> Result<bool> {
        if self.is_action(KeyAction::TabHosts, key) {
            self.active_tab = 0;
            return Ok(true);
        }
        if self.is_action(KeyAction::TabSftp, key) {
            self.switch_to_sftp_tab();
            return Ok(true);
        }
        if self.is_action(KeyAction::TabTunnels, key) {
            self.switch_to_tunnels_tab()?;
            return Ok(true);
        }
        if self.is_action(KeyAction::TabKeys, key) {
            self.switch_to_keys_tab()?;
            return Ok(true);
        }
        if self.is_action(KeyAction::TabAudit, key) {
            self.active_tab = 4;
            self.refresh_audit_events();
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn handle_key_palette(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => {
                self.mode = AppMode::Normal;
            }
            _ if self.is_action(KeyAction::Connect, &key) => {
                let chosen = self.palette_results.get(self.palette_selected).copied();
                self.mode = AppMode::Normal;
                if let Some(idx) = chosen {
                    if self.reveal_host(idx) {
                        self.connect_selected()?;
                    }
                }
            }
            // Plain letters are query text, even ones bound to nav (j/k/l). The
            // palette is a type-to-search field first; list navigation lives on
            // the arrow keys (handled below via KeyCode::Up/Down), so typing a
            // host name like "jira" is never eaten as a movement key.
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.palette_query.push(c);
                self.rebuild_palette_results();
            }
            _ if self.is_action(KeyAction::MoveUp, &key) => {
                if self.palette_selected > 0 {
                    self.palette_selected -= 1;
                }
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => {
                if self.palette_selected + 1 < self.palette_results.len() {
                    self.palette_selected += 1;
                }
            }
            KeyCode::Backspace => {
                self.palette_query.pop();
                self.rebuild_palette_results();
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn rebuild_palette_results(&mut self) {
        // nucleo fuzzy match (same engine as list search) — the palette is
        // advertised as fuzzy, so typos and abbreviations must match too.
        self.palette_results = self.search.update_query(&self.hosts, &self.palette_query);
        if self.palette_selected >= self.palette_results.len() {
            self.palette_selected = 0;
        }
    }

    pub(crate) fn handle_key_search(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => self.exit_search(true),
            _ if self.is_action(KeyAction::Connect, &key) => self.connect_selected()?,
            // Plain letters are query text, even ones bound to nav (j/k/l); list
            // navigation while searching lives on the arrow keys below.
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.search_query.push(c);
                self.rebuild_filter();
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_selection(1),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_selection(-1),
            KeyCode::Backspace => {
                self.search_query.pop();
                self.rebuild_filter();
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_key_host_detail(&mut self, key: KeyEvent) -> Result<()> {
        if self.detail_edit.is_none() {
            return Ok(());
        }
        let field = self.detail_edit.as_ref().unwrap().field;

        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => self.cancel_host_detail()?,
            _ if self.is_action(KeyAction::Connect, &key) => self.save_host_detail()?,
            _ if self.is_action(KeyAction::Favorite, &key) => self.toggle_favorite()?,
            _ if self.is_action(KeyAction::DetailFocus, &key) => self.detail_edit_field_next(),
            KeyCode::BackTab => self.detail_edit_field_prev(),
            _ if self.is_action(KeyAction::MoveDown, &key) => self.detail_edit_field_next(),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.detail_edit_field_prev(),
            KeyCode::Right if field.is_tri_state() => self.detail_edit_cycle_session_logging(1),
            KeyCode::Left if field.is_tri_state() => self.detail_edit_cycle_session_logging(-1),
            KeyCode::Char(' ')
                if key.modifiers.is_empty() && field == DetailEditField::SessionLogging =>
            {
                self.detail_edit_cycle_session_logging(1);
            }
            KeyCode::Backspace if key.modifiers.is_empty() => self.detail_edit_backspace(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control()
                    && !field.is_tri_state() =>
            {
                self.detail_edit_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_key_keychain(&mut self, key: KeyEvent) -> Result<()> {
        self.identity_notice = None;

        if self.try_tab_switch(&key)? {
            return Ok(());
        }

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::Cancel, &key) => {
                self.active_tab = 0;
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_identity_grid(1, 0),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_identity_grid(-1, 0),
            _ if self.is_action(KeyAction::MoveRight, &key) => self.move_identity_grid(0, 1),
            _ if self.is_action(KeyAction::MoveLeft, &key) => self.move_identity_grid(0, -1),
            _ if self.is_action(KeyAction::IdentityColumnsInc, &key) => {
                self.adjust_identity_columns(1);
            }
            _ if self.is_action(KeyAction::IdentityColumnsDec, &key) => {
                self.adjust_identity_columns(-1);
            }
            _ if self.is_action(KeyAction::AddHost, &key) => self.enter_identity_form(None)?,
            _ if self.is_action(KeyAction::Edit, &key) => self.edit_selected_identity()?,
            _ if self.is_action(KeyAction::Delete, &key) => self.delete_selected_identity()?,
            _ if self.is_action(KeyAction::RemoveFromAgent, &key) => {
                self.remove_selected_from_agent()?;
            }
            _ if self.is_action(KeyAction::AddToAgent, &key) => self.add_selected_to_agent()?,
            _ if self.is_action(KeyAction::Help, &key) => {
                self.pre_help_mode = Some(self.mode);
                self.mode = AppMode::Help;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_key_confirm_discard(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            _ if self.is_action(KeyAction::ConfirmYes, &key) => {
                // Save; on validation failure the form survives — return to it
                // so the user sees the notice instead of a stuck dialog.
                if self.host_form.is_some() {
                    self.save_host_form()?;
                    if self.host_form.is_some() && self.mode == AppMode::ConfirmDiscard {
                        self.mode = AppMode::HostForm;
                    }
                } else if self.identity_form.is_some() {
                    self.save_identity_form()?;
                    if self.identity_form.is_some() && self.mode == AppMode::ConfirmDiscard {
                        self.mode = AppMode::IdentityForm;
                    }
                } else if self.tunnel_form.is_some() {
                    self.save_tunnel_form()?;
                    if self.tunnel_form.is_some() && self.mode == AppMode::ConfirmDiscard {
                        self.mode = AppMode::TunnelForm;
                    }
                }
            }
            _ if self.is_action(KeyAction::ConfirmNo, &key) => {
                // Discard
                if self.host_form.is_some() {
                    self.discard_host_form()?;
                } else if self.identity_form.is_some() {
                    self.discard_identity_form()?;
                } else if self.tunnel_form.is_some() {
                    self.tunnel_form = None;
                    self.mode = AppMode::Normal;
                }
            }
            _ if self.is_action(KeyAction::Cancel, &key) => {
                // Go back to form
                if self.host_form.is_some() {
                    self.mode = AppMode::HostForm;
                } else if self.identity_form.is_some() {
                    self.mode = AppMode::IdentityForm;
                } else if self.tunnel_form.is_some() {
                    self.mode = AppMode::TunnelForm;
                } else {
                    self.mode = AppMode::Normal;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_key_help(&mut self, key: KeyEvent) -> Result<()> {
        if self.is_action(KeyAction::Cancel, &key)
            || self.is_action(KeyAction::Quit, &key)
            || self.is_action(KeyAction::Help, &key)
            || self.is_action(KeyAction::Connect, &key)
        {
            self.mode = self.pre_help_mode.take().unwrap_or(AppMode::Normal);
            self.help_scroll = 0;
            return Ok(());
        }
        // Ceiling = what the renderer can actually show, not the line count:
        // scrolling past it would silently bank presses that Up must unwind.
        let max = crate::tui::help_max_scroll(self.terminal_area);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.help_scroll = (self.help_scroll + 1).min(max);
            }
            KeyCode::PageUp => {
                self.help_scroll = self.help_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.help_scroll = (self.help_scroll + 10).min(max);
            }
            KeyCode::Home => self.help_scroll = 0,
            KeyCode::End => self.help_scroll = max,
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_key_confirm_delete(&mut self, key: KeyEvent) -> Result<()> {
        if self.is_action(KeyAction::ConfirmYes, &key) {
            match self.pending_delete.take() {
                Some(PendingDelete::Host { id, name }) => {
                    match self.store.delete_host(id)? {
                        DeleteHostOutcome::Deleted => {
                            self.host_notice = Some(format!("Host '{name}' deleted"));
                            self.reload_hosts()?;
                        }
                        DeleteHostOutcome::NotLauncher => {
                            self.host_notice = Some("Only launcher hosts can be deleted".into());
                        }
                        DeleteHostOutcome::NotFound => self.reload_hosts()?,
                    }
                    self.mode = AppMode::Normal;
                }
                Some(PendingDelete::Identity { id, name }) => {
                    match self.store.delete_identity(id)? {
                        crate::store::DeleteIdentityOutcome::Deleted => {
                            self.identity_notice = Some(format!("Identity '{name}' deleted"));
                            self.reload_identities()?;
                        }
                        crate::store::DeleteIdentityOutcome::InUse { host_count } => {
                            self.identity_notice = Some(format!(
                                "Cannot delete '{name}': used by {host_count} host(s)"
                            ));
                        }
                        crate::store::DeleteIdentityOutcome::NotFound => {
                            self.reload_identities()?;
                        }
                    }
                    self.mode = AppMode::Normal;
                }
                Some(PendingDelete::Group { id, name }) => {
                    if self.store.delete_group(id)? {
                        self.group_notice = Some(format!("Group '{name}' deleted"));
                        self.reload_hosts()?;
                    }
                    self.enter_group_manage()?;
                }
                Some(PendingDelete::Tunnel { id, label }) => {
                    let _ = self.tunnel_manager.stop_user(id);
                    self.tunnel_manager.clear_user_stopped(id);
                    self.store.delete_tunnel(id)?;
                    self.tunnel_notice = Some(format!("Tunnel '{label}' deleted"));
                    self.reload_tunnels()?;
                    self.mode = AppMode::Normal;
                }
                Some(PendingDelete::SftpEntry {
                    side, path, is_dir, ..
                }) => {
                    self.sftp_delete_confirmed(side, path, is_dir);
                    self.mode = AppMode::Normal;
                }
                None => {
                    self.mode = AppMode::Normal;
                }
            }
        } else if self.is_action(KeyAction::ConfirmNo, &key)
            || self.is_action(KeyAction::Cancel, &key)
        {
            let was_group = matches!(self.pending_delete, Some(PendingDelete::Group { .. }));
            self.pending_delete = None;
            if was_group {
                self.enter_group_manage()?;
            } else {
                self.mode = AppMode::Normal;
            }
        }
        Ok(())
    }

    pub(crate) fn exit_search(&mut self, reset_filter: bool) {
        self.search_query.clear();
        self.mode = AppMode::Normal;
        if reset_filter {
            self.tag_filters.clear();
        }
        self.rebuild_filter();
    }

    pub(crate) fn move_selection(&mut self, delta: i32) {
        if self.nav_rows.is_empty() {
            self.selected = 0;
            return;
        }
        let len = self.nav_rows.len() as i32;
        let next = self.selected as i32 + delta;
        // Wrap around: going past the end wraps to the beginning and vice versa
        self.selected = ((next % len + len) % len) as usize;
    }

    /// Jump the selection to the previous/next group header. When the cursor
    /// is on a host row, the jump is relative to that host's group. Wraps at
    /// both ends. No-op when there are no groups (flat host list).
    pub(crate) fn move_selection_by_group(&mut self, delta: i32) {
        if self.groups.is_empty() || self.nav_rows.is_empty() {
            return;
        }

        let header_positions: Vec<usize> = self
            .nav_rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| matches!(r, NavRow::Header(_)).then_some(i))
            .collect();
        if header_positions.is_empty() {
            return;
        }

        let current_group = match self.nav_rows.get(self.selected) {
            Some(NavRow::Header(si)) => Some(*si),
            Some(NavRow::Host(host_idx)) => self
                .group_sections
                .iter()
                .position(|s| s.host_indices.contains(host_idx)),
            None => None,
        };
        let current_group = current_group.unwrap_or(0);

        let current_header_idx = header_positions
            .iter()
            .position(
                |&pos| matches!(self.nav_rows[pos], NavRow::Header(si) if si == current_group),
            )
            .unwrap_or(0);

        let len = header_positions.len() as i32;
        let next = (current_header_idx as i32 + delta).rem_euclid(len) as usize;
        self.selected = header_positions[next];
    }

    /// Begin quitting: show the confirmation dialog, or quit immediately when
    /// confirmation is disabled in config.
    pub(crate) fn request_quit(&mut self) {
        if !self.config.appearance.confirm_quit {
            self.should_quit = true;
            return;
        }
        if self.mode != AppMode::ConfirmQuit {
            self.pre_quit_mode = Some(self.mode);
            self.mode = AppMode::ConfirmQuit;
        }
    }

    pub(crate) fn handle_key_confirm_quit(&mut self, key: KeyEvent) -> Result<()> {
        if self.is_action(KeyAction::ConfirmYes, &key) {
            self.should_quit = true;
        } else if self.is_action(KeyAction::ConfirmNo, &key)
            || self.is_action(KeyAction::Cancel, &key)
        {
            self.mode = self.pre_quit_mode.take().unwrap_or(AppMode::Normal);
        }
        Ok(())
    }

    pub(crate) fn handle_key_keybind_editor(&mut self, key: KeyEvent) -> Result<()> {
        let Some(editor) = self.keybind_editor else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        if editor.capturing {
            if key.code != KeyCode::Esc {
                if let Some(spec) = keyevent_to_spec(&key) {
                    let action = KeyAction::ALL[editor.selected];
                    if editor.append {
                        self.config.keybinds.add(action, spec);
                    } else {
                        self.config.keybinds.set(action, vec![spec]);
                    }
                    self.save_config_quietly();
                }
            }
            if let Some(e) = self.keybind_editor.as_mut() {
                e.capturing = false;
            }
            return Ok(());
        }

        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => {
                self.keybind_editor = None;
                self.mode = AppMode::Normal;
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.selected = (e.selected + 1) % KeyAction::ALL.len();
                    Self::clamp_keybind_editor_scroll(e);
                }
            }
            _ if self.is_action(KeyAction::MoveUp, &key) => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.selected = (e.selected + KeyAction::ALL.len() - 1) % KeyAction::ALL.len();
                    Self::clamp_keybind_editor_scroll(e);
                }
            }
            _ if self.is_action(KeyAction::Connect, &key) => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.capturing = true;
                    e.append = false;
                }
            }
            _ if self.is_action(KeyAction::AddHost, &key) => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.capturing = true;
                    e.append = true;
                }
            }
            KeyCode::Char('r') if key.modifiers.is_empty() => {
                let action = KeyAction::ALL[editor.selected];
                self.config.keybinds.reset_action(action);
                self.save_config_quietly();
            }
            KeyCode::Char('x') if key.modifiers.is_empty() => {
                let action = KeyAction::ALL[editor.selected];
                self.config.keybinds.set(action, Vec::new());
                self.save_config_quietly();
            }
            _ => {}
        }
        Ok(())
    }

    fn clamp_keybind_editor_scroll(editor: &mut KeybindEditor) {
        // Keep selection visible in a ~16-row viewport.
        const VIEWPORT: usize = 16;
        if editor.selected < editor.scroll {
            editor.scroll = editor.selected;
        } else if editor.selected >= editor.scroll + VIEWPORT {
            editor.scroll = editor.selected.saturating_sub(VIEWPORT - 1);
        }
    }

    /// Read the current value of the Settings toggle at row `i` (order matches
    /// [`SETTINGS_ITEMS`]).
    pub(crate) fn setting_value(&self, i: usize) -> bool {
        let a = &self.config.appearance;
        match i {
            0 => a.opaque_background,
            1 => a.os_logo,
            2 => a.confirm_quit,
            3 => a.disable_animation,
            4 => self.config.session_logging.enabled,
            _ => false,
        }
    }

    /// Flip the Settings toggle at row `i` and persist immediately.
    fn toggle_setting(&mut self, i: usize) {
        match i {
            0 => {
                self.config.appearance.opaque_background =
                    !self.config.appearance.opaque_background;
            }
            1 => self.config.appearance.os_logo = !self.config.appearance.os_logo,
            2 => self.config.appearance.confirm_quit = !self.config.appearance.confirm_quit,
            3 => {
                self.config.appearance.disable_animation =
                    !self.config.appearance.disable_animation;
            }
            4 => {
                self.config.session_logging.enabled = !self.config.session_logging.enabled;
            }
            _ => {}
        }
        self.save_config_quietly();
    }

    pub(crate) fn handle_key_settings(&mut self, key: KeyEvent) -> Result<()> {
        let n = SETTINGS_ITEMS.len();
        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => self.mode = AppMode::Normal,
            _ if self.is_action(KeyAction::MoveDown, &key) => {
                self.settings_selected = (self.settings_selected + 1) % n;
            }
            _ if self.is_action(KeyAction::MoveUp, &key) => {
                self.settings_selected = (self.settings_selected + n - 1) % n;
            }
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_setting(self.settings_selected),
            _ => {}
        }
        Ok(())
    }

    /// Persist config, surfacing failures as a non-fatal host notice.
    pub(crate) fn save_config_quietly(&mut self) {
        if let Err(e) = crate::config::save_config(&self.config) {
            self.host_notice = Some(format!("Could not save config: {e}"));
        }
    }

    /// Short human label of the configured save keys, e.g. `"F2/Ctrl+S"`,
    /// for form hints.
    pub fn save_key_label(&self) -> String {
        let keys = &self.config.keybinds.save;
        if keys.is_empty() {
            "F2".to_string()
        } else {
            keys.join("/")
        }
    }

    /// Whether `key` matches one of the user-configured bindings for `action`.
    pub fn is_action(&self, action: KeyAction, key: &KeyEvent) -> bool {
        self.config
            .keybinds
            .binds(action)
            .iter()
            .filter_map(|spec| parse_keyspec(spec))
            .any(|(code, mods)| keyspec_matches(code, mods, key))
    }

    /// Whether `key` matches the configured "save" binding (default F2/Ctrl+S).
    pub fn is_save_key(&self, key: &KeyEvent) -> bool {
        self.is_action(KeyAction::Save, key)
    }
}
