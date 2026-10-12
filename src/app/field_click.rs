//! Click to focus (#116): a plain left click on a form field focuses it and
//! puts the cursor under the pointer; on a picker row it highlights that row
//! without activating it, like moving there with the arrows.
//!
//! Renderers record what they drew where in `App::click_map`; this module
//! only reads that map, so a click always hits what the last frame showed.

use ratatui::layout::{Position, Rect};
use unicode_width::UnicodeWidthChar;

use super::*;

/// The char of a drawn value under display cell `cell`, counted from the
/// value's first cell. Each char takes its display width (one cell when the
/// value is `masked`), and the field being edited also draws the `_` cursor
/// glyph in one cell before char `glyph`.
fn char_at_cell(value: &str, cell: usize, glyph: Option<usize>, masked: bool) -> usize {
    let mut left = cell;
    for (i, ch) in value.chars().enumerate() {
        if glyph == Some(i) {
            if left == 0 {
                return i;
            }
            left -= 1;
        }
        let width = if masked { 1 } else { ch.width().unwrap_or(0) };
        if left < width {
            return i;
        }
        left -= width;
    }
    text_input::char_len(value)
}

impl App {
    /// Record a clickable region of the frame being drawn.
    pub(crate) fn record_click(&self, area: Rect, hit: ClickHit) {
        self.click_map.borrow_mut().push(ClickTarget { area, hit });
    }

    /// Record the visible rows of a one-row-per-item list drawn into `area`,
    /// scrolled so that item `offset` is on its first row.
    pub(crate) fn record_list_rows(&self, area: Rect, offset: usize, len: usize) {
        let shown = len.saturating_sub(offset).min(area.height as usize);
        for i in 0..shown {
            self.record_click(
                Rect::new(area.x, area.y + i as u16, area.width, 1),
                ClickHit::Row(offset + i),
            );
        }
    }

    /// Focus whatever the last frame drew at terminal cell (`col`, `row`).
    /// Clicks on chrome, or on a form under the active picker, do nothing.
    pub(crate) fn click_at(&mut self, col: u16, row: u16) {
        // Later records were drawn on top.
        let Some(target) = self
            .click_map
            .borrow()
            .iter()
            .rev()
            .find(|t| t.area.contains(Position::new(col, row)))
            .copied()
        else {
            return;
        };
        match target.hit {
            ClickHit::Field {
                index,
                value_x,
                start,
                end,
            } => {
                // Left of the value (the label) lands before its first char
                // on this row; past its end, after its last.
                let cell = (start + col.saturating_sub(value_x) as usize).min(end);
                self.focus_field(index, cell);
            }
            ClickHit::Row(index) => self.select_row(index),
        }
    }

    /// Focus field `index` (in its form's `ALL` order) with the cursor on the
    /// char drawn `cell` cells into its value, clamped to the value. Whether
    /// the click landed right of a `_` cursor glyph depends on the state the
    /// last frame was drawn from, so that is read before focus moves.
    fn focus_field(&mut self, index: usize, cell: usize) {
        let place = |value: &str, glyph: Option<usize>, masked: bool| {
            char_at_cell(value, cell, glyph, masked)
        };
        match self.mode {
            AppMode::HostForm => {
                let Some(form) = self.host_form.as_mut() else {
                    return;
                };
                let Some(&field) = HostFormField::ALL.get(index) else {
                    return;
                };
                let glyph = (form.field == field && form.editing).then_some(form.cursor);
                let masked = field == HostFormField::Password && !form.password_revealed;
                if form.field != field {
                    // Walking away re-masks, as with Tab.
                    form.password_revealed = false;
                    form.field = field;
                }
                form.cursor = place(form.active_field(), glyph, masked);
            }
            AppMode::IdentityForm => {
                let Some(form) = self.identity_form.as_mut() else {
                    return;
                };
                let Some(&field) = IdentityFormField::ALL.get(index) else {
                    return;
                };
                let glyph = (form.field == field).then_some(form.cursor);
                let masked = field == IdentityFormField::Password && !form.password_revealed;
                if form.field != field {
                    form.password_revealed = false;
                    form.field = field;
                }
                form.cursor = place(form.active_field(), glyph, masked);
            }
            AppMode::KeygenForm => {
                let Some(form) = self.keygen_form.as_mut() else {
                    return;
                };
                let Some(&field) = KeygenFormField::ALL.get(index) else {
                    return;
                };
                let glyph = (form.field == field).then_some(form.cursor);
                form.field = field;
                form.cursor = place(form.active_field(), glyph, false);
            }
            AppMode::GroupForm => {
                let Some(form) = self.group_form.as_mut() else {
                    return;
                };
                let Some(&field) = GroupFormField::ALL.get(index) else {
                    return;
                };
                // The group form draws no cursor glyph.
                form.field = field;
                form.cursor = match field {
                    GroupFormField::Name => place(&form.name, None, false),
                    GroupFormField::Username => place(&form.default_username, None, false),
                    GroupFormField::Port => place(&form.default_port, None, false),
                    GroupFormField::ProxyJump => place(&form.default_proxy_jump, None, false),
                    _ => 0,
                };
            }
            AppMode::TunnelForm => {
                let Some(form) = self.tunnel_form.as_mut() else {
                    return;
                };
                let Some(&field) = TunnelFormField::ALL.get(index) else {
                    return;
                };
                let glyph = (form.active_field == field).then_some(form.cursor);
                form.active_field = field;
                form.cursor = form
                    .active_text_field()
                    .map(|value| place(value, glyph, false))
                    .unwrap_or(0);
            }
            AppMode::SnippetForm => {
                let Some(form) = self.snippet_form.as_mut() else {
                    return;
                };
                let Some(&field) = SnippetFormField::ALL.get(index) else {
                    return;
                };
                let glyph = (form.field == field).then_some(form.cursor);
                form.field = field;
                form.cursor = place(Self::snippet_form_field_ref(form), glyph, false);
            }
            // Single-field prompts always draw the glyph.
            AppMode::SftpPrompt => {
                if let Some(prompt) = self.sftp_prompt.as_mut() {
                    prompt.cursor = place(&prompt.value, Some(prompt.cursor), false);
                }
            }
            AppMode::ImportPrompt => {
                if let Some(prompt) = self.import_prompt.as_mut() {
                    prompt.cursor = place(&prompt.path, Some(prompt.cursor), false);
                }
            }
            AppMode::BroadcastCommand => {
                if let Some(setup) = self.broadcast_setup.as_mut() {
                    setup.cursor = place(&setup.command, Some(setup.cursor), false);
                }
            }
            _ => {}
        }
    }

    /// Highlight row `index` of the active picker. Renderers only record rows
    /// that exist, so `index` is in range for the list that was drawn.
    fn select_row(&mut self, index: usize) {
        match self.mode {
            AppMode::FieldPicker => {
                if let Some(p) = self.field_picker.as_mut().filter(|p| p.creating.is_none()) {
                    p.selected = index;
                }
            }
            AppMode::GroupFieldPicker => {
                if let Some(p) = self.group_field_picker.as_mut() {
                    p.selected = index;
                }
            }
            AppMode::TunnelHostPicker => {
                if let Some(p) = self.tunnel_host_picker.as_mut() {
                    p.selected = index;
                }
            }
            AppMode::SessionPicker => {
                if let Some(p) = self.session_picker.as_mut() {
                    p.selected = index;
                }
            }
            AppMode::PushKeyHostPicker => {
                if let Some(p) = self.push_key_host_picker.as_mut() {
                    p.selected = index;
                }
            }
            AppMode::PushKeyIdentityPicker => {
                if let Some(p) = self.push_key_identity_picker.as_mut() {
                    p.selected = index;
                }
            }
            AppMode::SnippetPicker => {
                if let Some(p) = self.snippet_picker.as_mut() {
                    p.selected = index;
                }
            }
            AppMode::Palette => self.palette_selected = index,
            AppMode::TagFilter => self.tag_filter_selected = index,
            _ => {}
        }
    }
}
