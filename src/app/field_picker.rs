use super::*;

impl App {
    /// Visible dropdown rows as indexes into the full row list (groups then
    /// "+ New group…", or "(inherit from group)" then identities), fuzzy
    /// filtered by the picker's query, best match first. The special row
    /// stays pinned so it is reachable while filtering.
    pub fn field_picker_rows(&self) -> Vec<usize> {
        let Some(picker) = self.field_picker.as_ref() else {
            return Vec::new();
        };
        match picker.kind {
            PickerKind::Group => {
                let names: Vec<Vec<String>> =
                    self.groups.iter().map(|g| vec![g.name.clone()]).collect();
                let mut rows = crate::search::fuzzy_indices(&picker.query, &names);
                rows.push(self.field_picker_create_index());
                rows
            }
            PickerKind::Identity => {
                let names: Vec<Vec<String>> = self
                    .identities
                    .iter()
                    .map(|i| vec![i.name.clone()])
                    .collect();
                std::iter::once(0)
                    .chain(
                        crate::search::fuzzy_indices(&picker.query, &names)
                            .into_iter()
                            .map(|i| i + 1),
                    )
                    .collect()
            }
        }
    }

    /// Full-list row index of the highlighted visible row.
    fn field_picker_selected_row(&self) -> Option<usize> {
        let selected = self.field_picker.as_ref()?.selected;
        self.field_picker_rows().get(selected).copied()
    }

    /// Edit the filter and move the highlight to the best match (the first
    /// identity rather than the pinned "(inherit from group)" row).
    fn field_picker_edit_query(&mut self, edit: impl FnOnce(&mut String)) {
        let Some(p) = self.field_picker.as_mut() else {
            return;
        };
        edit(&mut p.query);
        let skip_inherit = p.kind == PickerKind::Identity && !p.query.is_empty();
        let rows = self.field_picker_rows().len();
        if let Some(p) = self.field_picker.as_mut() {
            p.selected = usize::from(skip_inherit && rows > 1);
        }
    }

    /// Index of the "+ New group…" row (Group picker only).
    pub(crate) fn field_picker_create_index(&self) -> usize {
        self.groups.len()
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
        // The query starts empty, so visible rows are the full list and the
        // stored full-list index is also the visible one.
        self.field_picker = Some(FieldPicker {
            kind,
            selected,
            query: String::new(),
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
        let len = self.field_picker_rows().len();
        match key.code {
            KeyCode::Esc => {
                self.field_picker = None;
                self.mode = AppMode::HostForm;
            }
            KeyCode::Down => {
                if let Some(p) = self.field_picker.as_mut() {
                    p.selected = (p.selected + 1) % len.max(1);
                }
            }
            KeyCode::Up => {
                if let Some(p) = self.field_picker.as_mut() {
                    p.selected = (p.selected + len.saturating_sub(1)) % len.max(1);
                }
            }
            KeyCode::Char(' ') if kind == PickerKind::Group && key.modifiers.is_empty() => {
                self.field_picker_toggle_group();
            }
            KeyCode::Enter => self.field_picker_confirm()?,
            KeyCode::Backspace => self.field_picker_edit_query(|q| {
                q.pop();
            }),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.field_picker_edit_query(|q| q.push(c));
            }
            _ => {}
        }
        Ok(())
    }

    /// Toggle the highlighted group's membership in the form's `group_ids`
    /// (Group picker, Space). The "+ New group…" row isn't a group and is a
    /// no-op here. Identity is left alone: group defaults resolve dynamically
    /// at connect time, so pinning today's default would only freeze the host
    /// against later group changes.
    pub(crate) fn field_picker_toggle_group(&mut self) {
        let Some(idx) = self.field_picker_selected_row() else {
            return;
        };
        let Some(group) = self.groups.get(idx) else {
            return; // create row
        };
        let gid = group.id;
        if let Some(form) = self.host_form.as_mut() {
            if form.group_ids.contains(&gid) {
                form.group_ids.remove(&gid);
            } else {
                form.group_ids.insert(gid);
            }
            form.dirty = true;
        }
    }

    pub(crate) fn field_picker_confirm(&mut self) -> Result<()> {
        let Some(picker) = self.field_picker.as_ref() else {
            return Ok(());
        };
        let kind = picker.kind;
        let Some(row) = self.field_picker_selected_row() else {
            return Ok(());
        };
        match kind {
            PickerKind::Group => {
                if row == self.field_picker_create_index() {
                    // Enter inline "new group" text entry.
                    if let Some(p) = self.field_picker.as_mut() {
                        p.creating = Some(String::new());
                        p.cursor = 0;
                    }
                    return Ok(());
                }
                // Multi-select: Space toggles rows; Enter adds the highlighted
                // group (never removes it) and closes the dropdown. Remember
                // the highlight so reopening lands on the same row.
                let gid = self.groups.get(row).map(|g| g.id);
                if let Some(form) = self.host_form.as_mut() {
                    form.group_index = row;
                    if gid.is_some_and(|gid| form.group_ids.insert(gid)) {
                        form.dirty = true;
                    }
                }
            }
            PickerKind::Identity => {
                if let Some(form) = self.host_form.as_mut() {
                    form.identity_index = row;
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
        let id = match self
            .store
            .list_groups()?
            .into_iter()
            .find(|g| g.name == name)
        {
            Some(g) => g.id,
            None => {
                self.store
                    .create_group(&crate::store::NewHostGroup {
                        name: name.clone(),
                        sort_order: self.groups.len() as i32,
                        ..Default::default()
                    })?
                    .id
            }
        };
        self.load_groups()?;
        if let Some(form) = self.host_form.as_mut() {
            // A freshly created group is added to the selection and highlighted.
            form.group_ids.insert(id);
            form.group_index = self.groups.iter().position(|g| g.id == id).unwrap_or(0);
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
        if !form.field.is_picker() && !form.field.is_toggle() && !form.field.is_tri_state() {
            return;
        }
        if form.field == HostFormField::ForwardAgent {
            form.forward_agent = match form.forward_agent {
                None => Some(true),
                Some(true) => Some(false),
                Some(false) => None,
            };
            form.dirty = true;
            return;
        }
        if form.field == HostFormField::Transport {
            form.transport = match form.transport {
                None => Some(crate::session_transport::SessionTransport::Ssh),
                Some(crate::session_transport::SessionTransport::Ssh) => {
                    Some(crate::session_transport::SessionTransport::Mosh)
                }
                Some(crate::session_transport::SessionTransport::Mosh) => None,
            };
            form.dirty = true;
            return;
        }
        if form.field == HostFormField::SessionLogging {
            form.session_logging = form.session_logging.step(delta);
            form.dirty = true;
            return;
        }
        if form.field == HostFormField::Ping {
            form.ping = form.ping.step(delta);
            form.dirty = true;
            return;
        }
        match form.field {
            // Group is multi-select — Left/Right no longer cycles a single
            // value; use Enter to open the checkbox dropdown (Space to toggle).
            HostFormField::Group => {}
            // Row 0 is "(inherit from group)"; rows 1..=len are the identities.
            HostFormField::Identity => {
                let max = self.identities.len();
                let next = form.identity_index as i32 + delta;
                form.identity_index = next.clamp(0, max as i32) as usize;
                form.dirty = true;
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
}
