use super::*;

impl App {
    /// Handle a keyboard event according to architecture keybindings.
    /// Handle a bracketed-paste event. Pasted text is delivered as one blob
    /// (not per-key), so multi-line content — e.g. a private key — no longer
    /// fires Enter/save mid-field and spills the rest as commands.
    pub fn handle_paste(&mut self, text: &str) -> Result<()> {
        // Embedded session: forward the paste straight to the remote PTY.
        if matches!(self.mode, AppMode::Session | AppMode::Connecting) {
            if let Some(s) = self.active_session_mut() {
                let _ = s.write_paste(text.as_bytes());
            }
            return Ok(());
        }

        // Only insert into modes that own a focused text field. Everywhere else
        // a paste is meaningless and must NOT be run as commands.
        let text_entry = matches!(
            self.mode,
            AppMode::HostForm
                | AppMode::IdentityForm
                | AppMode::GroupForm
                | AppMode::TunnelForm
                | AppMode::HostDetail
                | AppMode::Search
                | AppMode::TagFilter
                | AppMode::Palette
                | AppMode::ImportPrompt
        );
        if !text_entry {
            return Ok(());
        }

        // Pasting key material into the identity "Private key path" field:
        // keep the full multi-line blob and write it to a key file on save.
        if self.mode == AppMode::IdentityForm
            && crate::ssh::looks_like_private_key(text)
            && self
                .identity_form
                .as_ref()
                .is_some_and(|f| f.field == IdentityFormField::PrivateKey)
        {
            if let Some(form) = self.identity_form.as_mut() {
                form.pasted_key = Some(text.to_string());
                form.private_key = "(pasted key — saved to ~/.ssh on save)".to_string();
                form.cursor = text_input::char_len(&form.private_key);
                form.dirty = true;
            }
            return Ok(());
        }

        // Feed printable characters through the normal typing path (reusing the
        // field's insert logic); drop newlines/tabs since all fields are
        // single-line.
        for ch in text.chars() {
            if ch.is_control() {
                continue;
            }
            self.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()))?;
        }
        Ok(())
    }

    /// Handle a mouse event — clicks, scroll wheel, etc.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        if matches!(self.mode, AppMode::Connecting | AppMode::Session) {
            self.handle_mouse_session(mouse);
            return Ok(());
        }
        if self.mode != AppMode::Normal {
            return Ok(());
        }

        // When a dashboard panel is zoomed the whole body is that single panel,
        // so the 3-column routing below must not run (it would select a host
        // "through" the panel). Route the wheel to the zoomed panel's scroll and
        // swallow clicks.
        if self.panel_zoomed {
            match mouse.kind {
                MouseEventKind::ScrollDown => self.scroll_zoomed(true, 3),
                MouseEventKind::ScrollUp => self.scroll_zoomed(false, 3),
                _ => {}
            }
            return Ok(());
        }

        let areas =
            crate::tui::dashboard_layout::dashboard_layout_zoomed(self.terminal_area, self.ui_zoom);
        let x = mouse.column;
        let y = mouse.row;

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let now = std::time::Instant::now();
                let is_double = self
                    .last_click
                    .map(|(t, lx, ly)| {
                        now.duration_since(t).as_millis() < 400
                            && x.abs_diff(lx) <= 1
                            && y.abs_diff(ly) <= 1
                    })
                    .unwrap_or(false);
                self.last_click = Some((now, x, y));

                // Tab bar clicks
                if y == areas.tab_bar.y {
                    if let Some(tab) = tab_from_x(x) {
                        match tab {
                            0 => self.active_tab = 0,
                            1 => self.switch_to_sftp_tab(),
                            2 => self.switch_to_tunnels_tab()?,
                            3 => self.switch_to_keys_tab()?,
                            4 => {
                                self.active_tab = 4;
                                self.refresh_audit_events();
                            }
                            _ => {}
                        }
                    }
                    return Ok(());
                }

                // Body area clicks
                if y >= areas.body.y && y < areas.body.y + areas.body.height {
                    match self.active_tab {
                        0 => {
                            // Host list panel
                            if x >= areas.col_left.x && x < areas.col_left.x + areas.col_left.width
                            {
                                let content_y = areas.col_left.y + 1;
                                // Mirror the panel's body height: ch = height-2,
                                // body reserves 2 rows for its footer.
                                let ch = areas.col_left.height.saturating_sub(2);
                                let body_h = if ch > 2 { ch - 2 } else { ch } as usize;
                                if y >= content_y {
                                    let rel = y - content_y;
                                    if let Some(idx) = self.host_row_to_index(rel, body_h) {
                                        if let Some(pos) = self
                                            .nav_rows
                                            .iter()
                                            .position(|r| matches!(r, NavRow::Host(i) if *i == idx))
                                        {
                                            self.selected = pos;
                                            if is_double {
                                                self.connect_selected()?;
                                            }
                                        }
                                    } else if let Some(si) = self.host_row_to_header(rel, body_h) {
                                        // Click on a group header toggles collapse.
                                        self.toggle_group_by_section(si);
                                    }
                                }
                            }
                        }
                        1 => {
                            // SFTP tab — no body-click interaction yet.
                        }
                        2 => {
                            // Tunnels table — account for scroll offset
                            let data_y = areas.body.y + 4;
                            if y >= data_y {
                                let visible_row = (y - data_y) as usize;
                                let max_rows = (areas.body.y + areas.body.height)
                                    .saturating_sub(data_y)
                                    as usize;
                                let scroll = if self.tunnel_selected >= max_rows {
                                    self.tunnel_selected - max_rows + 1
                                } else {
                                    0
                                };
                                let idx = scroll + visible_row;
                                if idx < self.tunnels.len() {
                                    self.tunnel_selected = idx;
                                    if is_double {
                                        self.toggle_tunnel()?;
                                    }
                                }
                            }
                        }
                        3 => {
                            // Keys cards
                            if !self.identities.is_empty() {
                                let inner_w =
                                    crate::tui::screens::keys::inner_width(areas.body.width);
                                let cards_per_row = crate::tui::screens::keys::resolve_columns(
                                    inner_w,
                                    self.config.appearance.identity_columns,
                                );
                                let card_h = 6u16;
                                let rel_y = y.saturating_sub(areas.body.y);
                                let row_idx = rel_y / (card_h + 1);
                                let col_idx = if cards_per_row > 1 {
                                    let card_w = inner_w
                                        .saturating_sub((cards_per_row as u16 - 1) * 2)
                                        / cards_per_row as u16;
                                    let margin = if areas.body.width >= 132 {
                                        2
                                    } else if areas.body.width >= 80 {
                                        1
                                    } else {
                                        0
                                    };
                                    let rel_x = x.saturating_sub(areas.body.x + margin);
                                    (rel_x / (card_w + 2)).min(cards_per_row as u16 - 1)
                                } else {
                                    0
                                };
                                let row_offset = self.keys_scroll_row_offset(
                                    areas.body.height,
                                    cards_per_row,
                                    card_h + 1,
                                );
                                let idx = (row_idx as usize + row_offset) * cards_per_row
                                    + col_idx as usize;
                                if idx < self.identities.len() {
                                    self.identity_selected = idx;
                                }
                            }
                        }
                        4 => {
                            // Audit table (mirror the renderer's scroll math)
                            let data_y = areas.body.y + 3;
                            if y >= data_y {
                                let max_rows = (areas.body.y + areas.body.height)
                                    .saturating_sub(data_y)
                                    as usize;
                                let scroll = if max_rows > 0 && self.audit_selected >= max_rows {
                                    self.audit_selected - max_rows + 1
                                } else {
                                    0
                                };
                                let row = (y - data_y) as usize + scroll;
                                if row < self.auth_events_cache.len() {
                                    self.audit_selected = row;
                                }
                            }

                            // Filter strip clicks (row 0 of audit area)
                            if y == areas.body.y {
                                self.handle_audit_filter_click(x, areas.body.x)?;
                            }
                        }
                        _ => {}
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                if y >= areas.body.y && y < areas.body.y + areas.body.height {
                    match self.active_tab {
                        0 => {
                            if x >= areas.col_left.x && x < areas.col_left.x + areas.col_left.width
                            {
                                self.selected = self.selected.saturating_sub(3);
                            } else {
                                self.ssh_log_scroll = self.ssh_log_scroll.saturating_add(3);
                            }
                        }
                        1 => {}
                        2 => {
                            self.tunnel_selected = self.tunnel_selected.saturating_sub(1);
                        }
                        3 => {
                            self.identity_selected = self.identity_selected.saturating_sub(1);
                        }
                        4 => {
                            self.audit_selected = self.audit_selected.saturating_sub(1);
                        }
                        _ => {}
                    }
                }
            }
            MouseEventKind::ScrollDown
                if y >= areas.body.y && y < areas.body.y + areas.body.height =>
            {
                match self.active_tab {
                    0 => {
                        if x >= areas.col_left.x && x < areas.col_left.x + areas.col_left.width {
                            // `selected` indexes nav_rows (group headers + hosts),
                            // not filtered_indices (hosts only) — clamping to the
                            // shorter list moved the selection up and hid rows.
                            let max = self.nav_rows.len().saturating_sub(1);
                            self.selected = (self.selected + 3).min(max);
                        } else {
                            self.ssh_log_scroll = self.ssh_log_scroll.saturating_sub(3);
                        }
                    }
                    1 => {}
                    2 => {
                        let max = self.tunnels.len().saturating_sub(1);
                        self.tunnel_selected = (self.tunnel_selected + 1).min(max);
                    }
                    3 => {
                        let max = self.identities.len().saturating_sub(1);
                        self.identity_selected = (self.identity_selected + 1).min(max);
                    }
                    4 => {
                        let max = self.auth_events_cache.len().saturating_sub(1);
                        self.audit_selected = (self.audit_selected + 1).min(max);
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        Ok(())
    }
}
