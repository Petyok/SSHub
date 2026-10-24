use ratatui::layout::{Position, Rect};

use super::*;

impl App {
    /// Handle a keyboard event according to architecture keybindings.
    /// Handle a bracketed-paste event. Pasted text is delivered as one blob
    /// (not per-key), so multi-line content — e.g. a private key — no longer
    /// fires Enter/save mid-field and spills the rest as commands.
    pub fn handle_paste(&mut self, text: &str) -> Result<()> {
        if self.mode == AppMode::AuthPrompt {
            self.paste_auth(text);
            return Ok(());
        }
        // Embedded session: forward the paste straight to the remote PTY.
        if matches!(self.mode, AppMode::Session | AppMode::Connecting) {
            if let Some(s) = self.active_session_mut() {
                s.observe_paste(text);
                if s.write_paste(text.as_bytes()).is_err() {
                    s.history.input.invalidate();
                }
            }
            return Ok(());
        }

        if self.mode == AppMode::ProfilePicker {
            if let Some(picker) = self.profile_picker.as_mut() {
                picker.handle_paste(text);
            }
            return Ok(());
        }

        // The map tab's filter line and host picker take pasted text too.
        if self.map_paste(text) {
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
                | AppMode::SessionPicker
                | AppMode::ImportPrompt
                | AppMode::SnippetForm
                | AppMode::SnippetPicker
                | AppMode::SftpPrompt
        ) || (self.mode == AppMode::Normal
            && matches!(self.active_tab, 2 | 3)
            && self.list_filter_editing);
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
            // A drag begun over an overlay that closed mid-gesture must not
            // resurface as a highlight on the next one.
            self.panel_sel = None;
            self.handle_mouse_session(mouse);
            return Ok(());
        }
        // Forms, dialogs and pickers: drag to select and copy what is on
        // screen; a plain click focuses what is under the pointer (#116).
        if is_overlay_mode(self.mode) {
            if let Some((col, row)) = self.mouse_select(mouse) {
                self.click_at(col, row);
            }
            return Ok(());
        }

        // When a dashboard panel is zoomed the whole body is that single panel,
        // so the 3-column routing below must not run (it would select a host
        // "through" the panel). Route the wheel to the zoomed panel's scroll and
        // swallow clicks.
        if self.panel_zoomed && self.active_tab == 0 {
            match mouse.kind {
                MouseEventKind::ScrollDown => self.scroll_zoomed(true, 3),
                MouseEventKind::ScrollUp => self.scroll_zoomed(false, 3),
                _ => {
                    self.mouse_select(mouse);
                }
            }
            return Ok(());
        }
        let areas =
            crate::tui::dashboard_layout::dashboard_layout_zoomed(self.terminal_area, self.ui_zoom);
        let x = mouse.column;
        let y = mouse.row;

        match mouse.kind {
            // A drag selects whatever is on screen and copies it on release;
            // a press and release on one cell is a click on what is there.
            MouseEventKind::Down(MouseButton::Left)
            | MouseEventKind::Drag(MouseButton::Left)
            | MouseEventKind::Up(MouseButton::Left) => {
                if let Some((x, y)) = self.mouse_select(mouse) {
                    self.dashboard_click(x, y, &areas)?;
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
                        1 => self.sftp_wheel(x, y, areas.body, -3),
                        2 => self.move_tunnel_selection(-1),
                        3 => self.step_identity_selection(-1),
                        4 => {
                            self.audit_selected = self.audit_selected.saturating_sub(1);
                        }
                        5 => self.map_wheel(-3),
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
                    1 => self.sftp_wheel(x, y, areas.body, 3),
                    2 => self.move_tunnel_selection(1),
                    3 => self.step_identity_selection(1),
                    4 => {
                        let max = self.auth_events_cache.len().saturating_sub(1);
                        self.audit_selected = (self.audit_selected + 1).min(max);
                    }
                    5 => self.map_wheel(3),
                    _ => {}
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// A click on the dashboard: a tab, a panel, a row.
    fn dashboard_click(
        &mut self,
        x: u16,
        y: u16,
        areas: &crate::tui::dashboard_layout::DashboardAreas,
    ) -> Result<()> {
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
        let tunnel_pick = self.tunnel_click_pick.take();

        // Tab bar clicks
        if y == areas.tab_bar.y {
            let has_profile = self.profile.as_ref().is_some_and(|p| !p.compat);
            if let Some(tab) = crate::tui::widgets::tab_bar::tab_at(x, areas.tab_bar, has_profile) {
                self.reset_tab_input_state();
                match tab {
                    0 => self.active_tab = 0,
                    1 => self.switch_to_sftp_tab(),
                    2 => self.switch_to_tunnels_tab()?,
                    3 => self.switch_to_keys_tab()?,
                    4 => {
                        self.active_tab = 4;
                        self.refresh_audit_events();
                    }
                    5 => self.switch_to_map_tab(),
                    _ => {}
                }
            }
            return Ok(());
        }

        if !areas.body.contains(Position::new(x, y)) {
            return Ok(());
        }
        match self.active_tab {
            0 => {
                // The clicked panel takes the focus (what `z` zooms).
                if let Some(panel) = self.dashboard_panel_at(areas, x, y) {
                    self.focused_panel = panel;
                }
                if areas.col_left.contains(Position::new(x, y))
                    && self.click_hosts_panel(areas.col_left, y)
                    && is_double
                {
                    self.connect_selected()?;
                }
            }
            1 => self.sftp_click(x, y, areas.body, is_double)?,
            5 => self.map_click(x, y, is_double)?,
            2 => {
                // A double click acts on the tunnel its first click picked:
                // picking one can show or hide its detail line, which moves
                // the table under the pointer between the two clicks.
                let picked = tunnel_pick
                    .filter(|_| is_double)
                    .and_then(|id| self.tunnels.iter().position(|t| t.id == id));
                let view = self.tunnel_view();
                if let Some(idx) = picked {
                    self.tunnel_selected = idx;
                    return self.toggle_tunnel();
                }
                // Tunnels table — account for scroll offset
                let data_y = crate::tui::screens::tunnels::data_top(areas.body, self);
                if y >= data_y {
                    let visible_row = (y - data_y) as usize;
                    let max_rows =
                        (areas.body.y + areas.body.height).saturating_sub(data_y) as usize;
                    let sel_pos = view
                        .iter()
                        .position(|&i| i == self.tunnel_selected)
                        .unwrap_or(0);
                    let scroll = if sel_pos >= max_rows {
                        sel_pos - max_rows + 1
                    } else {
                        0
                    };
                    if let Some(&idx) = view.get(scroll + visible_row) {
                        self.tunnel_selected = idx;
                        self.tunnel_click_pick = Some(self.tunnels[idx].id);
                    }
                }
            }
            3 => {
                // Keys cards
                let view = self.identity_view();
                if !view.is_empty() {
                    let inner_w = crate::tui::screens::keys::inner_width(areas.body.width);
                    let cards_per_row = crate::tui::screens::keys::resolve_columns(
                        inner_w,
                        self.config.appearance.identity_columns,
                    );
                    let card_h = 6u16;
                    let rel_y = y.saturating_sub(areas.body.y);
                    // Below the card grid are the agent strip and the filter bar.
                    let grid_h = crate::tui::screens::keys::grid_height(areas.body.height);
                    if rel_y >= grid_h {
                        return Ok(());
                    }
                    let row_idx = rel_y / (card_h + 1);
                    let col_idx = if cards_per_row > 1 {
                        let card_w = inner_w.saturating_sub((cards_per_row as u16 - 1) * 2)
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
                    let row_offset = self.keys_scroll_row_offset(grid_h, cards_per_row, card_h + 1);
                    let pos = (row_idx as usize + row_offset) * cards_per_row + col_idx as usize;
                    if let Some(&idx) = view.get(pos) {
                        self.identity_selected = idx;
                    }
                }
            }
            4 => {
                // Audit table (mirror the renderer's scroll math)
                let data_y = crate::tui::screens::audit::data_top(areas.body, self);
                if y >= data_y {
                    let max_rows =
                        (areas.body.y + areas.body.height).saturating_sub(data_y) as usize;
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
        Ok(())
    }

    /// The hosts-tab panel at (`x`, `y`): the docked broadcast panel first,
    /// since it is drawn over the grid.
    fn dashboard_panel_at(
        &self,
        areas: &crate::tui::dashboard_layout::DashboardAreas,
        x: u16,
        y: u16,
    ) -> Option<PanelId> {
        let docked = self.broadcast.is_some().then_some(PanelId::Broadcast);
        docked
            .into_iter()
            .chain([
                PanelId::Hosts,
                PanelId::Detail,
                PanelId::Agent,
                PanelId::Latency,
                PanelId::Recent,
                PanelId::Auth,
                PanelId::Ping,
                PanelId::SshLog,
            ])
            .find(|&p| crate::tui::panel_zoom_source(areas, p).contains(Position::new(x, y)))
    }

    /// A click on row `y` of the hosts panel drawn in `panel` (the hosts tab,
    /// or the SFTP picker): select the host there, or fold the group header.
    /// True when a host row was hit, so a double click can act on it.
    fn click_hosts_panel(&mut self, panel: Rect, y: u16) -> bool {
        let content_y = panel.y + 1;
        if y < content_y {
            return false;
        }
        // Mirror the panel's body height: ch = height-2, body reserves 2 rows
        // for its footer.
        let ch = panel.height.saturating_sub(2);
        let body_h = if ch > 2 { ch - 2 } else { ch } as usize;
        let rel = y - content_y;
        if let Some(idx) = self.host_row_to_index(rel, body_h) {
            if let Some(pos) = self
                .nav_rows
                .iter()
                .position(|r| matches!(r, NavRow::Host(i) if *i == idx))
            {
                self.selected = pos;
                return true;
            }
        } else if let Some(si) = self.host_row_to_header(rel, body_h) {
            self.toggle_group_by_section(si);
        }
        false
    }

    /// A click on the SFTP tab. The picker is the hosts panel (all of the body
    /// but its hint line); in the browser a click focuses the pane under it
    /// and picks the entry there, and a double click opens it like Enter.
    fn sftp_click(&mut self, x: u16, y: u16, body: Rect, is_double: bool) -> Result<()> {
        if self.sftp.is_none() {
            let list = crate::tui::dashboard_layout::sftp_picker_rect(body);
            if list.contains(Position::new(x, y)) && self.click_hosts_panel(list, y) && is_double {
                self.sftp_picker_activate()?;
            }
            return Ok(());
        }
        let Some((pane, pos)) = self.sftp_pane_at(x, y) else {
            return Ok(());
        };
        if let Some(s) = self.sftp.as_mut() {
            s.focus = pane;
            if let Some(pos) = pos {
                s.focused_pane_mut().selected = pos;
            }
        }
        if is_double && pos.is_some() {
            self.sftp_enter_selected();
        }
        Ok(())
    }

    /// The wheel on the SFTP tab: the picker list, or the pane under the
    /// pointer (without moving the focus).
    fn sftp_wheel(&mut self, x: u16, y: u16, body: Rect, delta: i64) {
        if self.sftp.is_none() {
            if body.contains(Position::new(x, y)) {
                let max = self.nav_rows.len().saturating_sub(1) as i64;
                self.selected = (self.selected as i64 + delta).clamp(0, max) as usize;
            }
            return;
        }
        if let Some((pane, _)) = self.sftp_pane_at(x, y) {
            if let Some(s) = self.sftp.as_mut() {
                s.pane_mut(pane).move_selection(delta);
            }
        }
    }

    /// The SFTP browser pane the last frame drew at (`x`, `y`), and the entry
    /// on that row, if any.
    fn sftp_pane_at(&self, x: u16, y: u16) -> Option<(crate::sftp::model::Focus, Option<usize>)> {
        self.click_map
            .borrow()
            .iter()
            .rev()
            .find(|t| t.area.contains(Position::new(x, y)))
            .and_then(|t| match t.hit {
                ClickHit::SftpPane { pane, pos } => Some((pane, pos)),
                _ => None,
            })
    }

    /// Terminal-style drag selection over whatever is rendered (the zoomed
    /// panel, or the whole screen anywhere else outside a session). The render
    /// pass highlights the span and scrapes it into `panel_sel_text`; a drag is
    /// copied on release. Returns the position of a left click that never moved,
    /// which is a click, not a one-cell selection.
    fn mouse_select(&mut self, mouse: MouseEvent) -> Option<(u16, u16)> {
        let at = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.panel_sel = Some(PanelSel {
                    anchor: at,
                    cur: at,
                });
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(sel) = self.panel_sel.as_mut() {
                    sel.cur = at;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let sel = self.panel_sel.take()?;
                if sel.anchor == sel.cur {
                    return Some(sel.cur);
                }
                let text = self.panel_sel_text.borrow().clone();
                if !text.trim().is_empty() {
                    let chars = text.chars().count();
                    let notice = match write_osc52(&text) {
                        Ok(()) => format!("copied {chars} chars to clipboard"),
                        Err(e) => format!("clipboard copy failed: {e:#}"),
                    };
                    // A picker or auth prompt over a live session is drawn on
                    // the session view, which shows only the session's notice.
                    if self.session_is_rendered() {
                        if let Some(session) = self.active_session_mut() {
                            session.set_copy_notice(notice);
                        }
                    } else {
                        self.host_notice = Some(notice);
                    }
                }
            }
            _ => {}
        }
        None
    }
}
