use super::*;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        // When an embedded session is active, Ctrl+C inside the terminal must
        // reach the remote shell — not quit sshub. Session mode intercepts all
        // keys (except Ctrl+D / Esc, which end the session) before this check.
        if matches!(self.mode, AppMode::Connecting | AppMode::Session) {
            return self.handle_key_session(key);
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
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

        // Ctrl+K opens the keybinding editor from any normal navigation screen.
        if self.mode == AppMode::Normal
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('k') | KeyCode::Char('K'))
        {
            self.keybind_editor = Some(KeybindEditor {
                selected: 0,
                capturing: false,
                append: false,
            });
            self.mode = AppMode::KeybindEditor;
            return Ok(());
        }

        match self.mode {
            AppMode::KeybindEditor => self.handle_key_keybind_editor(key),
            AppMode::ConfirmQuit => self.handle_key_confirm_quit(key),
            AppMode::Help => self.handle_key_help(key),
            AppMode::ConfirmDiscard => self.handle_key_confirm_discard(key),
            AppMode::ConfirmDelete => self.handle_key_confirm_delete(key),
            AppMode::HostForm => self.handle_key_host_form(key),
            AppMode::IdentityForm => self.handle_key_identity_form(key),
            AppMode::GroupForm => self.handle_key_group_form(key),
            AppMode::GroupIdentityPicker => self.handle_key_group_identity_picker(key),
            AppMode::TunnelHostPicker => self.handle_key_tunnel_host_picker(key),
            AppMode::FieldPicker => self.handle_key_field_picker(key),
            AppMode::ImportPrompt => self.handle_key_import_prompt(key),
            AppMode::GroupManage => self.handle_key_group_manage(key),
            AppMode::Palette => self.handle_key_palette(key),
            AppMode::Search => self.handle_key_search(key),
            AppMode::TagFilter => self.handle_key_tag_filter(key),
            AppMode::HostDetail => self.handle_key_host_detail(key),
            AppMode::TunnelForm => self.handle_key_tunnel_form(key),
            AppMode::Connecting | AppMode::Session => self.handle_key_session(key),
            AppMode::Normal => match self.active_tab {
                1 => self.handle_key_tunnels(key),
                2 => self.handle_key_keychain(key),
                3 => self.handle_key_audit(key),
                _ => self.handle_key_normal(key),
            },
        }
    }

    pub(crate) fn handle_key_normal(&mut self, key: KeyEvent) -> Result<()> {
        self.host_notice = None;

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_host_manual(-1)?
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_host_manual(1)?
            }
            KeyCode::Char('j') | KeyCode::Down if key.modifiers.is_empty() => {
                self.move_selection(1)
            }
            KeyCode::Char('k') | KeyCode::Up if key.modifiers.is_empty() => self.move_selection(-1),
            KeyCode::Esc if key.modifiers.is_empty() && !self.tag_filters.is_empty() => {
                self.tag_filters.clear();
                self.search_query.clear();
                self.rebuild_filter();
            }
            // Collapse/expand the group under the selection.
            KeyCode::Char(' ') if key.modifiers.is_empty() => self.toggle_selected_group(),
            KeyCode::Left if key.modifiers.is_empty() => {
                if self.selected_nav_header().is_some_and(|si| {
                    !self.group_sections[si].collapsed
                }) {
                    self.toggle_selected_group();
                }
            }
            KeyCode::Right if key.modifiers.is_empty() => {
                if self
                    .selected_nav_header()
                    .is_some_and(|si| self.group_sections[si].collapsed)
                {
                    self.toggle_selected_group();
                }
            }
            KeyCode::Char('Z') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                // Collapse all, or expand all if everything is already collapsed.
                let all_collapsed = !self.group_sections.is_empty()
                    && self.group_sections.iter().all(|s| s.collapsed);
                self.set_all_groups_collapsed(!all_collapsed);
            }
            // Enter on a group header toggles it; on a host it connects.
            KeyCode::Enter if self.selected_nav_header().is_some() => {
                self.toggle_selected_group()
            }
            KeyCode::Enter => self.connect_selected()?,
            _ if self.is_action(KeyAction::AddHost, &key) => self.enter_host_form(None, false)?,
            _ if self.is_action(KeyAction::Delete, &key) => self.delete_selected_host()?,
            _ if self.is_action(KeyAction::Duplicate, &key) => self.duplicate_selected_host()?,
            KeyCode::Char('E') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                match self.export_ssh_config() {
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
                }
            }
            KeyCode::Char('I') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                match self.import_ssh_config() {
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
                }
            }
            KeyCode::Char('T') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.open_import_prompt();
            }
            // `e` on a group header configures its default identity; on a host
            // it edits the host.
            KeyCode::Char('e') if key.modifiers.is_empty() => {
                if self.selected_nav_header().is_some() {
                    self.open_group_identity_picker()?;
                } else {
                    self.edit_selected_host()?;
                }
            }
            // Host-name column zoom: widen (`+`/`=`) or narrow (`-`/`_`).
            KeyCode::Char('+' | '=')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.set_ui_zoom((self.ui_zoom + 1).min(UI_ZOOM_MAX));
            }
            KeyCode::Char('-' | '_')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.set_ui_zoom(self.ui_zoom.saturating_sub(1));
            }
            KeyCode::Char('f') if key.modifiers.is_empty() => self.toggle_favorite()?,
            KeyCode::Tab => self.detail_focus = !self.detail_focus,
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
            _ if self.is_action(KeyAction::TagFilter, &key) => {
                self.open_tag_filter();
            }
            KeyCode::Char('c') if key.modifiers.is_empty() => {
                self.ssh_log.clear();
                self.ssh_log_scroll = 0;
                // The periodic ssh probe was removed — the receiver is left
                // around for the type signature but never produces anything.
                self.probe_rx = None;
                self.host_notice = Some("SSH log cleared.".into());
            }
            KeyCode::Char('i') if key.modifiers.is_empty() => self.switch_to_keys_tab()?,
            KeyCode::Char('h') if key.modifiers.is_empty() => self.active_tab = 0,
            KeyCode::Char('1') if key.modifiers.is_empty() => self.active_tab = 0,
            KeyCode::Char('2') if key.modifiers.is_empty() => self.switch_to_tunnels_tab()?,
            KeyCode::Char('3') if key.modifiers.is_empty() => self.switch_to_keys_tab()?,
            KeyCode::Char('4') if key.modifiers.is_empty() => {
                self.active_tab = 3;
                self.refresh_audit_events();
            }
            KeyCode::Char('s') if key.modifiers.is_empty() => self.cycle_sort_mode(),
            KeyCode::Char('y') if key.modifiers.is_empty() => self.yank_ssh_log()?,
            KeyCode::Char('g' | 'G')
                if key
                    .modifiers
                    .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
            {
                self.delete_selected_host_group()?
            }
            KeyCode::Char('g' | 'G')
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.enter_group_manage()?
            }
            KeyCode::Char('g' | 'G')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.rename_selected_host_group()?
            }
            // Unmatched chars open the fuzzy palette instead of legacy search
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.palette_query.clear();
                self.palette_query.push(c);
                self.palette_selected = 0;
                self.rebuild_palette_results();
                self.mode = AppMode::Palette;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_key_palette(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Enter => {
                let chosen = self.palette_results.get(self.palette_selected).copied();
                self.mode = AppMode::Normal;
                if let Some(idx) = chosen {
                    // Reveal (and select) the exact host chosen, then connect.
                    // `reveal_host` clears any filter that hides it and expands
                    // its group, so we never connect to a different host.
                    if self.reveal_host(idx) {
                        self.connect_selected()?;
                    }
                }
            }
            KeyCode::Up => {
                if self.palette_selected > 0 {
                    self.palette_selected -= 1;
                }
            }
            KeyCode::Down => {
                if self.palette_selected + 1 < self.palette_results.len() {
                    self.palette_selected += 1;
                }
            }
            KeyCode::Backspace => {
                self.palette_query.pop();
                self.rebuild_palette_results();
            }
            KeyCode::Char(c) if !c.is_control() => {
                self.palette_query.push(c);
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
            KeyCode::Esc => self.exit_search(true),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Enter => self.connect_selected()?,
            KeyCode::Backspace => {
                self.search_query.pop();
                self.rebuild_filter();
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.search_query.push(c);
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

        match key.code {
            KeyCode::Esc => self.cancel_host_detail()?,
            KeyCode::Enter => self.save_host_detail()?,
            KeyCode::Char('f') if key.modifiers.is_empty() => self.toggle_favorite()?,
            KeyCode::Tab if key.modifiers.is_empty() => self.detail_edit_field_next(),
            KeyCode::BackTab => self.detail_edit_field_prev(),
            KeyCode::Char('j') | KeyCode::Down if key.modifiers.is_empty() => {
                self.detail_edit_field_next()
            }
            KeyCode::Char('k') | KeyCode::Up if key.modifiers.is_empty() => {
                self.detail_edit_field_prev()
            }
            KeyCode::Backspace if key.modifiers.is_empty() => self.detail_edit_backspace(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.detail_edit_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_key_keychain(&mut self, key: KeyEvent) -> Result<()> {
        self.identity_notice = None;

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            KeyCode::Char('h') if key.modifiers.is_empty() => {
                self.active_tab = 0;
            }
            KeyCode::Char('1') if key.modifiers.is_empty() => {
                self.active_tab = 0;
            }
            KeyCode::Char('2') if key.modifiers.is_empty() => self.switch_to_tunnels_tab()?,
            KeyCode::Char('3') if key.modifiers.is_empty() => self.switch_to_keys_tab()?,
            KeyCode::Char('4') if key.modifiers.is_empty() => {
                self.active_tab = 3;
                self.refresh_audit_events();
            }
            KeyCode::Esc if key.modifiers.is_empty() => {
                self.active_tab = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_identity_grid(1, 0),
            KeyCode::Char('k') | KeyCode::Up => self.move_identity_grid(-1, 0),
            KeyCode::Char('l') | KeyCode::Right => self.move_identity_grid(0, 1),
            KeyCode::Left => self.move_identity_grid(0, -1),
            KeyCode::Char(']') if key.modifiers.is_empty() => self.adjust_identity_columns(1),
            KeyCode::Char('[') if key.modifiers.is_empty() => self.adjust_identity_columns(-1),
            KeyCode::Char('a') if key.modifiers.is_empty() => self.enter_identity_form(None)?,
            KeyCode::Char('e') if key.modifiers.is_empty() => self.edit_selected_identity()?,
            KeyCode::Char('d') if key.modifiers.is_empty() => self.delete_selected_identity()?,
            KeyCode::Char('r') if key.modifiers.is_empty() => self.remove_selected_from_agent()?,
            KeyCode::Char('p') if key.modifiers.is_empty() => self.add_selected_to_agent()?,
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
            KeyCode::Char('y') if key.modifiers.is_empty() => {
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
            KeyCode::Char('n') if key.modifiers.is_empty() => {
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
            KeyCode::Esc => {
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
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Enter => {
                self.mode = self.pre_help_mode.take().unwrap_or(AppMode::Normal);
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_key_confirm_delete(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') => match self.pending_delete.take() {
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
                    if self.tunnel_manager.is_running(id) {
                        self.tunnel_manager.stop(id)?;
                    }
                    self.store.delete_tunnel(id)?;
                    self.tunnel_notice = Some(format!("Tunnel '{label}' deleted"));
                    self.reload_tunnels()?;
                    self.mode = AppMode::Normal;
                }
                None => {
                    self.mode = AppMode::Normal;
                }
            },
            KeyCode::Char('n') | KeyCode::Esc => {
                let was_group = matches!(self.pending_delete, Some(PendingDelete::Group { .. }));
                self.pending_delete = None;
                if was_group {
                    self.enter_group_manage()?;
                } else {
                    self.mode = AppMode::Normal;
                }
            }
            _ => {}
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
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                self.should_quit = true;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.mode = self.pre_quit_mode.take().unwrap_or(AppMode::Normal);
            }
            _ => {}
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
            KeyCode::Esc | KeyCode::Char('q') => {
                self.keybind_editor = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.selected = (e.selected + 1) % KeyAction::ALL.len();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.selected =
                        (e.selected + KeyAction::ALL.len() - 1) % KeyAction::ALL.len();
                }
            }
            // Enter/c: replace with a single new key. a: add another binding.
            KeyCode::Enter | KeyCode::Char('c') => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.capturing = true;
                    e.append = false;
                }
            }
            KeyCode::Char('a') => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.capturing = true;
                    e.append = true;
                }
            }
            KeyCode::Char('r') => {
                let action = KeyAction::ALL[editor.selected];
                self.config.keybinds.reset_action(action);
                self.save_config_quietly();
            }
            KeyCode::Char('x') => {
                // Unbind the action entirely.
                let action = KeyAction::ALL[editor.selected];
                self.config.keybinds.set(action, Vec::new());
                self.save_config_quietly();
            }
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
