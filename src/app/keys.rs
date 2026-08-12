use super::*;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.mode == AppMode::SessionPicker {
            return self.handle_key_session_picker(key);
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

        // Open a local shell tab from any dashboard tab.
        if matches!(self.mode, AppMode::Normal) && self.is_action(KeyAction::LocalShell, &key) {
            self.open_local_shell()?;
            return Ok(());
        }

        // Keybinding editor from the dashboard navigation screens.
        if self.mode == AppMode::Normal && self.is_action(KeyAction::KeybindEditor, &key) {
            self.keybind_editor = Some(KeybindEditor {
                selected: 0,
                scroll: 0,
                capturing: false,
                append: false,
                query: String::new(),
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

        // Session-strip binds (resume / switch / tabs / new tab / …) must work on
        // every dashboard tab — the footer advertises them whenever sessions
        // exist. The switcher goes through the same door rather than its own.
        if self.mode == AppMode::Normal && self.handle_key_background_sessions(&key) {
            return Ok(());
        }

        match self.mode {
            AppMode::KeybindEditor => self.handle_key_keybind_editor(key),
            AppMode::Settings => self.handle_key_settings(key),
            AppMode::ThemePicker => self.handle_key_theme_picker(key),
            AppMode::TunnelReconnectSettings => self.handle_key_tunnel_reconnect_settings(key),
            AppMode::ConfirmQuit => self.handle_key_confirm_quit(key),
            AppMode::Help => self.handle_key_help(key),
            AppMode::ConfirmDiscard => self.handle_key_confirm_discard(key),
            AppMode::ConfirmDelete => self.handle_key_confirm_delete(key),
            AppMode::HostForm => self.handle_key_host_form(key),
            AppMode::IdentityForm => self.handle_key_identity_form(key),
            AppMode::KeygenForm => self.handle_key_keygen_form(key),
            AppMode::GroupForm => self.handle_key_group_form(key),
            AppMode::GroupFieldPicker => self.handle_key_group_field_picker(key),
            AppMode::TunnelHostPicker => self.handle_key_tunnel_host_picker(key),
            AppMode::SessionPicker => self.handle_key_session_picker(key),
            AppMode::PushKeyHostPicker => self.handle_key_push_key_host_picker(key),
            AppMode::PushKeyIdentityPicker => self.handle_key_push_key_identity_picker(key),
            AppMode::FieldPicker => self.handle_key_field_picker(key),
            AppMode::ImportPrompt => self.handle_key_import_prompt(key),
            AppMode::SftpPrompt => self.handle_key_sftp_prompt(key),
            AppMode::GroupManage => self.handle_key_group_manage(key),
            AppMode::Palette => self.handle_key_palette(key),
            AppMode::Search => self.handle_key_search(key),
            AppMode::TagFilter => self.handle_key_tag_filter(key),
            AppMode::HostDetail => self.handle_key_host_detail(key),
            AppMode::TunnelForm => self.handle_key_tunnel_form(key),
            AppMode::BroadcastPickTarget => self.handle_key_broadcast_pick(key),
            AppMode::BroadcastCommand => self.handle_key_broadcast_command(key),
            AppMode::BroadcastPreview => self.handle_key_broadcast_preview(key),
            AppMode::Notice => self.handle_key_notice(key),
            AppMode::KnownHosts => self.handle_key_known_hosts(key),
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

        if self.try_tab_switch(&key)? {
            return Ok(());
        }

        // Broadcast (#3). The cancel key does double duty, claimed before any
        // other binding: cancel a live run, or (nothing running) clear the error
        // toasts. Works regardless of focus, matching the always-shown footer
        // hint. `open_broadcast` refuses a second concurrent run.
        if self.is_action(KeyAction::BroadcastCancel, &key)
            && (self.broadcast.is_some() || !self.broadcast_toasts.is_empty())
        {
            if self
                .broadcast
                .as_ref()
                .is_some_and(|b| !crate::broadcast::all_terminal(&b.results))
            {
                self.cancel_broadcast();
            } else {
                self.broadcast_toasts.clear();
            }
            return Ok(());
        }
        if self.is_action(KeyAction::Broadcast, &key) {
            self.open_broadcast();
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
                self.scroll_zoomed(true, step);
            }
            _ if self.panel_zoomed
                && self.focused_panel != PanelId::Hosts
                && (self.is_action(KeyAction::MoveUp, &key) || key.code == KeyCode::PageUp) =>
            {
                let step = if key.code == KeyCode::PageUp { 10 } else { 1 };
                self.scroll_zoomed(false, step);
            }
            // Connect to the host selected in a zoomed ping/recent panel.
            _ if self.panel_zoomed
                && matches!(self.focused_panel, PanelId::Ping | PanelId::Recent)
                && self.is_action(KeyAction::Connect, &key) =>
            {
                self.connect_zoomed_host()?;
            }
            // Zoomed auth panel behaves like the Audit tab: cycle status / range.
            _ if self.panel_zoomed
                && self.focused_panel == PanelId::Auth
                && self.is_action(KeyAction::AuditFilter, &key) =>
            {
                self.audit_filter = self.audit_filter.next();
                self.refresh_audit_events();
            }
            _ if self.panel_zoomed
                && self.focused_panel == PanelId::Auth
                && self.is_action(KeyAction::AuditRange, &key) =>
            {
                self.audit_range = self.audit_range.next();
                self.refresh_audit_events();
            }
            // Zoomed agent panel: remove the selected key from ssh-agent.
            _ if self.panel_zoomed
                && self.focused_panel == PanelId::Agent
                && self.is_action(KeyAction::Delete, &key) =>
            {
                self.remove_zoomed_agent_key();
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_selection(1),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_selection(-1),
            _ if self.is_action(KeyAction::Cancel, &key) && self.panel_zoomed => {
                self.panel_zoomed = false;
                self.panel_scroll.set(0);
                self.panel_sel = None;
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
                let to_zoomed = !self.panel_zoomed;
                self.panel_zoomed = to_zoomed;
                self.panel_scroll.set(0);
                self.panel_sel = None;
                // Morph the panel between its grid slot and the full body (#35).
                // Broadcast is a floating panel with its own animation path.
                self.zoom_anim =
                    if self.motion_enabled() && self.focused_panel != PanelId::Broadcast {
                        let areas = crate::tui::dashboard_layout::dashboard_layout_zoomed(
                            self.terminal_area,
                            self.ui_zoom,
                        );
                        let slot = crate::tui::panel_zoom_source(&areas, self.focused_panel);
                        let (from, to) = if to_zoomed {
                            (slot, areas.body)
                        } else {
                            (areas.body, slot)
                        };
                        Some(crate::tui::tween::SlideAnim::new_in_out(
                            from,
                            to,
                            std::time::Duration::from_millis(320),
                        ))
                    } else {
                        None
                    };
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
                self.palette_adhoc = self.compute_palette_adhoc();
                self.mode = AppMode::Palette;
            }
            _ if self.is_action(KeyAction::Help, &key) => {
                self.open_help();
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
            _ if self.is_action(KeyAction::PushKey, &key) => self.trigger_push_key_from_hosts()?,
            _ => {}
        }
        Ok(())
    }

    /// Modal message popup (`AppMode::Notice`): any key dismisses it back to the
    /// dashboard. Used e.g. for an SFTP connection error.
    pub(crate) fn handle_key_notice(&mut self, _key: KeyEvent) -> Result<()> {
        self.notice_popup = None;
        self.mode = AppMode::Normal;
        Ok(())
    }

    /// Move dashboard panel focus one step in `dir`; a no-op at a grid edge.
    fn focus_panel(&mut self, dir: FocusDir) {
        // A zoomed panel is exclusive (tmux-style): don't move focus while
        // zoomed, or the zoomed view would swap panels under the user.
        if self.panel_zoomed {
            return;
        }
        if let Some(next) = self.focused_panel.neighbor(dir) {
            // The Broadcast panel only exists while a run is live (#3); its
            // neighbor edges are always present in the grid, so skip the move
            // when there's nothing to focus.
            if next == PanelId::Broadcast && self.broadcast.is_none() {
                return;
            }
            self.focused_panel = next;
            self.panel_scroll.set(0);
            self.panel_sel = None;
        }
    }

    /// Best-effort removal of the key selected in a zoomed agent panel (issue
    /// #18). `ssh-add -d` can only drop a key by file path, and the agent
    /// listing exposes only the key's comment (usually, but not always, that
    /// path), so this can fail with a clear notice.
    fn remove_zoomed_agent_key(&mut self) {
        let agent = crate::ssh::agent::detect_agent();
        let idx = self.panel_scroll.get() as usize;
        let Some(k) = agent.keys.get(idx) else {
            return;
        };
        if k.comment.is_empty() {
            self.host_notice = Some("can't remove: agent key has no file path".into());
            return;
        }
        self.host_notice = Some(match crate::ssh::agent::remove_key(&k.comment) {
            Ok(()) => format!("removed {} from agent", k.comment),
            Err(_) => format!(
                "couldn't remove {} (ssh-add needs the key file path)",
                k.comment
            ),
        });
    }

    /// Connect to the host selected in a zoomed ping/recent panel (issue #18).
    fn connect_zoomed_host(&mut self) -> Result<()> {
        let idx = self.panel_scroll.get() as usize;
        let host_idx = self.zoomed_host_idx.borrow().get(idx).copied();
        if let Some(hi) = host_idx {
            self.connect_host_at(hi)?;
        }
        Ok(())
    }

    /// Scroll the focused zoomed panel (issue #18) by `step` rows. `down` moves
    /// toward the end of a list / the latest ssh-log line. Shared by the
    /// keyboard arms and the mouse wheel.
    pub(crate) fn scroll_zoomed(&mut self, down: bool, step: u16) {
        if self.focused_panel == PanelId::SshLog {
            // The ssh-log offset counts back from the latest line, so scrolling
            // "down" (toward the latest) decreases it.
            self.ssh_log_scroll = if down {
                self.ssh_log_scroll.saturating_sub(step as usize)
            } else {
                self.ssh_log_scroll.saturating_add(step as usize)
            };
        } else {
            let cur = self.panel_scroll.get();
            self.panel_scroll.set(if down {
                cur.saturating_add(step)
            } else {
                cur.saturating_sub(step)
            });
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
                if self.palette_adhoc.is_some()
                    && self.palette_selected == self.palette_results.len()
                {
                    let t = self.palette_adhoc.take().unwrap();
                    return self.connect_adhoc(t);
                }
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
                let total = self.palette_results.len() + self.palette_adhoc.is_some() as usize;
                if self.palette_selected + 1 < total {
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
        self.palette_adhoc = self.compute_palette_adhoc();
        let total = self.palette_results.len() + self.palette_adhoc.is_some() as usize;
        if total == 0 || self.palette_selected >= total {
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
            _ if self.is_action(KeyAction::GenerateKey, &key) => self.enter_keygen_form()?,
            _ if self.is_action(KeyAction::Edit, &key) => self.edit_selected_identity()?,
            _ if self.is_action(KeyAction::Delete, &key) => self.delete_selected_identity()?,
            _ if self.is_action(KeyAction::RemoveFromAgent, &key) => {
                self.remove_selected_from_agent()?;
            }
            _ if self.is_action(KeyAction::AddToAgent, &key) => self.add_selected_to_agent()?,
            _ if self.is_action(KeyAction::PushKey, &key) => self.trigger_push_key_from_keys()?,
            _ if self.is_action(KeyAction::Help, &key) => {
                self.open_help();
            }
            _ if self.is_action(KeyAction::KnownHosts, &key) => {
                self.open_known_hosts();
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
                } else if self.keygen_form.is_some() {
                    self.save_keygen_form()?;
                    if self.keygen_form.is_some() && self.mode == AppMode::ConfirmDiscard {
                        self.mode = AppMode::KeygenForm;
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
                } else if self.keygen_form.is_some() {
                    self.discard_keygen_form()?;
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
                } else if self.keygen_form.is_some() {
                    self.mode = AppMode::KeygenForm;
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

    pub(crate) fn open_help(&mut self) {
        self.pre_help_mode = Some(self.mode);
        self.mode = AppMode::Help;
        self.help_scroll = 0;
        self.help_query.clear();
    }

    pub(crate) fn handle_key_help(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                if !self.help_query.is_empty() {
                    self.help_query.clear();
                    self.help_scroll = 0;
                } else {
                    self.mode = self.pre_help_mode.take().unwrap_or(AppMode::Normal);
                    self.help_scroll = 0;
                }
            }
            // Enter is not printable query input; keep it as dismiss when idle.
            KeyCode::Enter if self.help_query.is_empty() => {
                self.mode = self.pre_help_mode.take().unwrap_or(AppMode::Normal);
                self.help_scroll = 0;
            }
            // Ceiling = what the renderer can actually show, not the line count:
            // scrolling past it would silently bank presses that Up must unwind.
            KeyCode::Up => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                let max = crate::tui::help_max_scroll(self.terminal_area, &self.help_query);
                self.help_scroll = (self.help_scroll + 1).min(max);
            }
            KeyCode::PageUp => {
                self.help_scroll = self.help_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                let max = crate::tui::help_max_scroll(self.terminal_area, &self.help_query);
                self.help_scroll = (self.help_scroll + 10).min(max);
            }
            KeyCode::Home => self.help_scroll = 0,
            KeyCode::End => {
                self.help_scroll =
                    crate::tui::help_max_scroll(self.terminal_area, &self.help_query);
            }
            KeyCode::Backspace => {
                self.help_query.pop();
                self.help_scroll = 0;
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.help_query.push(c);
                self.help_scroll = 0;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn open_known_hosts(&mut self) {
        let path = crate::known_hosts::known_hosts_path();
        let mut state = KnownHostsState {
            entries: Vec::new(),
            selected: 0,
            query: String::new(),
            confirming_delete: false,
            notice: None,
            notice_is_error: false,
        };
        match crate::known_hosts::load_known_hosts(&path) {
            Ok(entries) => state.entries = entries,
            Err(e) => {
                state.notice = Some(format!("Could not read {}: {e}", path.display()));
                state.notice_is_error = true;
            }
        }
        self.known_hosts = Some(state);
        self.mode = AppMode::KnownHosts;
    }

    pub(crate) fn handle_key_known_hosts(&mut self, key: KeyEvent) -> Result<()> {
        let is_yes = self.is_action(KeyAction::ConfirmYes, &key);
        let delete = self.is_action(KeyAction::KnownHostsDelete, &key);
        let refresh = self.is_action(KeyAction::KnownHostsRefresh, &key);
        let Some(state) = self.known_hosts.as_mut() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        if state.confirming_delete {
            if is_yes {
                let filtered = state.filtered_indices();
                if let Some(&fi) = filtered.get(state.selected) {
                    let entry = state.entries[fi].clone();
                    let path = crate::known_hosts::known_hosts_path();
                    match crate::known_hosts::remove_host(&entry.hosts, &path) {
                        Ok(()) => match crate::known_hosts::load_known_hosts(&path) {
                            Ok(entries) => {
                                state.entries = entries;
                                state.selected = 0;
                                state.notice =
                                    Some(format!("Removed all keys for {}", entry.hosts));
                                state.notice_is_error = false;
                            }
                            Err(e) => {
                                state.notice = Some(format!(
                                    "Removed keys for {}, but reload failed: {e}",
                                    entry.hosts
                                ));
                                state.notice_is_error = true;
                            }
                        },
                        Err(e) => {
                            state.notice = Some(format!("{e}"));
                            state.notice_is_error = true;
                        }
                    }
                }
                state.confirming_delete = false;
            } else {
                state.confirming_delete = false;
            }
            return Ok(());
        }

        if delete {
            state.notice = None;
            let filtered = state.filtered_indices();
            if let Some(&fi) = filtered.get(state.selected) {
                if let Some(reason) = state.entries[fi].deletion_block_reason() {
                    state.notice = Some(reason.to_string());
                    state.notice_is_error = true;
                } else {
                    state.confirming_delete = true;
                }
            }
            return Ok(());
        }
        if refresh {
            let path = crate::known_hosts::known_hosts_path();
            match crate::known_hosts::load_known_hosts(&path) {
                Ok(entries) => {
                    state.entries = entries;
                    state.selected = 0;
                    state.notice = Some("Refreshed".to_string());
                    state.notice_is_error = false;
                }
                Err(e) => {
                    state.notice = Some(format!("Refresh failed: {e}"));
                    state.notice_is_error = true;
                }
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Esc => {
                if !state.query.is_empty() {
                    state.query.clear();
                    state.selected = 0;
                } else {
                    self.known_hosts = None;
                    self.mode = AppMode::Normal;
                }
            }
            KeyCode::Up => {
                state.selected = state.selected.saturating_sub(1);
                state.notice = None;
            }
            KeyCode::Down => {
                let filtered = state.filtered_indices();
                if state.selected + 1 < filtered.len() {
                    state.selected += 1;
                }
                state.notice = None;
            }
            KeyCode::PageUp => {
                state.selected = state.selected.saturating_sub(10);
                state.notice = None;
            }
            KeyCode::PageDown => {
                let filtered = state.filtered_indices();
                state.selected = (state.selected + 10).min(filtered.len().saturating_sub(1));
                state.notice = None;
            }
            KeyCode::Backspace => {
                state.query.pop();
                state.selected = 0;
                state.notice = None;
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                state.query.push(c);
                state.selected = 0;
                state.notice = None;
            }
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
                            let credential_cleanup = self
                                .password_store
                                .delete(&crate::credentials::host_key(id))
                                .err();
                            self.host_notice = Some(format!("Host '{name}' deleted"));
                            if let Some(err) = credential_cleanup {
                                self.host_notice = Some(format!(
                                    "Host '{name}' deleted; credential cleanup failed: {err}"
                                ));
                            }
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
                            let credential_cleanup = self
                                .password_store
                                .delete(&crate::credentials::identity_key(id))
                                .err();
                            self.identity_notice = Some(format!("Identity '{name}' deleted"));
                            if let Some(err) = credential_cleanup {
                                self.identity_notice = Some(format!(
                                    "Identity '{name}' deleted; credential cleanup failed: {err}"
                                ));
                            }
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
                    self.tunnel_manager.stop_user(id)?;
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
        let Some(editor) = self.keybind_editor.clone() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        if editor.capturing {
            if key.code != KeyCode::Esc {
                if let Some(spec) = keyevent_to_spec(&key) {
                    // `selected` indexes the filtered list — rebinding ALL[selected]
                    // would silently edit the wrong action under an active filter.
                    let actions = self.filtered_keybind_actions();
                    if let Some(&action) = actions.get(editor.selected) {
                        if editor.append {
                            self.config.keybinds.add(action, spec);
                        } else {
                            self.config.keybinds.set(action, vec![spec]);
                        }
                        self.save_config_quietly();
                    }
                }
            }
            if let Some(e) = self.keybind_editor.as_mut() {
                e.capturing = false;
            }
            // Rebinding can drop the row out of a bind-text filter; keep selection
            // inside the new list so Enter/Ctrl+R/Ctrl+X don't go silent.
            self.clamp_keybind_editor_selection();
            return Ok(());
        }

        match key.code {
            KeyCode::Esc => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    if !e.query.is_empty() {
                        e.query.clear();
                        e.selected = 0;
                        e.scroll = 0;
                    } else {
                        self.keybind_editor = None;
                        self.mode = AppMode::Normal;
                    }
                }
            }
            KeyCode::Down => {
                let len = self.filtered_keybind_actions().len();
                if len > 0 {
                    if let Some(e) = self.keybind_editor.as_mut() {
                        e.selected = (e.selected + 1) % len;
                        Self::clamp_keybind_editor_scroll(e);
                    }
                }
            }
            KeyCode::Up => {
                let len = self.filtered_keybind_actions().len();
                if len > 0 {
                    if let Some(e) = self.keybind_editor.as_mut() {
                        e.selected = (e.selected + len - 1) % len;
                        Self::clamp_keybind_editor_scroll(e);
                    }
                }
            }
            KeyCode::PageDown => {
                let len = self.filtered_keybind_actions().len();
                if len > 0 {
                    if let Some(e) = self.keybind_editor.as_mut() {
                        e.selected = (e.selected + 10).min(len - 1);
                        Self::clamp_keybind_editor_scroll(e);
                    }
                }
            }
            KeyCode::PageUp => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.selected = e.selected.saturating_sub(10);
                    Self::clamp_keybind_editor_scroll(e);
                }
            }
            KeyCode::Enter => {
                let has_rows = !self.filtered_keybind_actions().is_empty();
                if let Some(e) = self.keybind_editor.as_mut() {
                    if has_rows {
                        e.capturing = true;
                        e.append = false;
                    }
                }
            }
            // Ctrl+letter row actions so unmodified letters stay query input.
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                let has_rows = !self.filtered_keybind_actions().is_empty();
                if let Some(e) = self.keybind_editor.as_mut() {
                    if has_rows {
                        e.capturing = true;
                        e.append = true;
                    }
                }
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
                let actions = self.filtered_keybind_actions();
                if let Some(&action) = actions.get(editor.selected) {
                    self.config.keybinds.reset_action(action);
                    self.save_config_quietly();
                    self.clamp_keybind_editor_selection();
                }
            }
            KeyCode::Char('x') if key.modifiers == KeyModifiers::CONTROL => {
                let actions = self.filtered_keybind_actions();
                if let Some(&action) = actions.get(editor.selected) {
                    self.config.keybinds.set(action, Vec::new());
                    self.save_config_quietly();
                    self.clamp_keybind_editor_selection();
                }
            }
            KeyCode::Backspace => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.query.pop();
                    e.selected = 0;
                    e.scroll = 0;
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.query.push(c);
                    e.selected = 0;
                    e.scroll = 0;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Actions visible in the keybinding editor under the current filter query.
    pub fn filtered_keybind_actions(&self) -> Vec<KeyAction> {
        let query = self
            .keybind_editor
            .as_ref()
            .map(|e| e.query.to_lowercase())
            .unwrap_or_default();
        KeyAction::ALL
            .iter()
            .copied()
            .filter(|action| {
                query.is_empty()
                    || action.label().to_lowercase().contains(&query)
                    || self
                        .config
                        .keybinds
                        .binds(*action)
                        .iter()
                        .any(|b| b.to_lowercase().contains(&query))
            })
            .collect()
    }

    fn clamp_keybind_editor_selection(&mut self) {
        let len = self.filtered_keybind_actions().len();
        if let Some(e) = self.keybind_editor.as_mut() {
            if len == 0 {
                e.selected = 0;
                e.scroll = 0;
            } else if e.selected >= len {
                e.selected = len - 1;
            }
            Self::clamp_keybind_editor_scroll(e);
        }
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

    /// Current value of a Settings row: `Some` for a boolean toggle, `None`
    /// for an action row like [`SettingItem::Theme`], which has no value.
    pub(crate) fn setting_value(&self, item: impl Into<SettingItem>) -> Option<bool> {
        let a = &self.config.appearance;
        match item.into() {
            SettingItem::Theme => None,
            SettingItem::Toggle(t) => Some(match t {
                SettingToggle::TransparentSshubBackground => a.transparent_sshub_background,
                SettingToggle::TransparentSessionBackground => a.transparent_session_background,
                SettingToggle::OsLogo => a.os_logo,
                SettingToggle::ConfirmQuit => a.confirm_quit,
                SettingToggle::DisableAnimation => a.disable_animation,
                SettingToggle::SessionLogging => self.config.session_logging.enabled,
            }),
        }
    }

    /// Flip a boolean Settings row, reporting whether anything changed. Action
    /// rows such as [`SettingItem::Theme`] are a no-op and return `false`.
    ///
    /// Persisting is the caller's job (see [`App::handle_key_settings`]): a
    /// pure flip keeps the row semantics testable without writing a config
    /// file, and it keeps an action row from touching config.toml at all.
    pub(crate) fn toggle_setting(&mut self, item: impl Into<SettingItem>) -> bool {
        let SettingItem::Toggle(toggle) = item.into() else {
            return false;
        };
        match toggle {
            SettingToggle::TransparentSshubBackground => {
                self.config.appearance.transparent_sshub_background =
                    !self.config.appearance.transparent_sshub_background;
            }
            SettingToggle::TransparentSessionBackground => {
                self.config.appearance.transparent_session_background =
                    !self.config.appearance.transparent_session_background;
            }
            SettingToggle::OsLogo => {
                self.config.appearance.os_logo = !self.config.appearance.os_logo
            }
            SettingToggle::ConfirmQuit => {
                self.config.appearance.confirm_quit = !self.config.appearance.confirm_quit
            }
            SettingToggle::DisableAnimation => {
                self.config.appearance.disable_animation =
                    !self.config.appearance.disable_animation;
            }
            SettingToggle::SessionLogging => {
                self.config.session_logging.enabled = !self.config.session_logging.enabled;
            }
        }
        true
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
            KeyCode::Enter
                if matches!(
                    SETTINGS_ITEMS.get(self.settings_selected).map(|d| d.item),
                    Some(SettingItem::Theme)
                ) =>
            {
                self.open_theme_picker();
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                // Action rows ignore both keys here: nothing flips, nothing is
                // written. Space on the Theme row stays a deliberate no-op —
                // only Enter opens the picker.
                let item = SETTINGS_ITEMS.get(self.settings_selected).map(|d| d.item);
                if item.is_some_and(|item| self.toggle_setting(item)) {
                    self.save_config_quietly();
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Theme picker keys (spec, "Theme-Picker / Interaktion"). Nothing here
    /// writes to disk except `Enter`.
    pub(crate) fn handle_key_theme_picker(&mut self, key: KeyEvent) -> Result<()> {
        // The page step is what the renderer can actually show, taken from the
        // same pure geometry function the renderer uses, so a page never jumps
        // further than the eye can follow.
        let page = crate::tui::screens::theme_picker::visible_rows(self.terminal_area).max(1);
        let last = self.theme_picker_rows().len().saturating_sub(1);
        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => self.cancel_theme_picker(),
            KeyCode::Enter => self.commit_theme_picker(),
            KeyCode::Char('r') => self.reload_theme_picker(),
            KeyCode::Home => {
                self.select_theme_row(0);
            }
            KeyCode::End => {
                self.select_theme_row(last);
            }
            KeyCode::PageUp => self.page_theme_selection(-(page as isize)),
            KeyCode::PageDown => self.page_theme_selection(page as isize),
            // The two-row footer cannot hold every ignored role at once, so it
            // scrolls. Shift is what keeps these off the navigation keys, whose
            // bindings match modifiers exactly.
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_theme_diagnostics(1)
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_theme_diagnostics(-1)
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_theme_selection(1),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_theme_selection(-1),
            _ => {}
        }
        Ok(())
    }

    /// The `config.toml` this run reads and writes: the profile's own file when
    /// a profile resolved startup, else the global one.
    ///
    /// Startup loads exactly this file (`lib.rs`), so a writer that picks a
    /// different one saves where nothing will ever read: the setting appears to
    /// take, and the next start silently serves the old value. Every persist
    /// path goes through here for that reason.
    pub(crate) fn config_target(&self) -> Option<std::path::PathBuf> {
        self.profile.as_ref().map(|p| p.config_file.clone())
    }

    /// Persist config, surfacing failures as a non-fatal host notice.
    pub(crate) fn save_config_quietly(&mut self) {
        let result = save_config_to(self.config_target().as_deref(), &self.config);
        if let Err(e) = result {
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

/// Write `config` to the file this run owns: the profile's `config.toml`, or the
/// global one in compatibility mode. The single place that chooses, so no call
/// site can pick the file startup does not read.
pub(crate) fn save_config_to(
    target: Option<&std::path::Path>,
    config: &AppConfig,
) -> anyhow::Result<()> {
    match target {
        Some(path) => crate::config::save_config_at(path, config),
        None => crate::config::save_config(config),
    }
}
