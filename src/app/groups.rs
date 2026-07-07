use super::*;

impl App {
    pub(crate) fn selected_host_group_id(&self) -> Option<i64> {
        // Prefer the selected host's group; fall back to a selected group header
        // so "add host" while sitting on a group header files it in that group.
        self.selected_entry()
            .and_then(|e| e.managed())
            .and_then(|m| m.group_id)
            .or_else(|| {
                self.selected_nav_header()
                    .and_then(|si| self.group_sections.get(si))
                    .and_then(|section| section.group.as_ref())
                    .map(|g| g.id)
            })
    }

    pub(crate) fn enter_group_manage(&mut self) -> Result<()> {
        self.groups = self.store.list_groups()?;
        self.group_notice = None;
        self.clamp_group_manage_selected();
        self.mode = AppMode::GroupManage;
        Ok(())
    }

    pub(crate) fn clamp_group_manage_selected(&mut self) {
        if !self.groups.is_empty() {
            self.group_manage_selected = self.group_manage_selected.min(self.groups.len() - 1);
        } else {
            self.group_manage_selected = 0;
        }
    }

    pub(crate) fn move_group_manage_selection(&mut self, delta: isize) {
        if self.groups.is_empty() {
            return;
        }
        let new = self.group_manage_selected as isize + delta;
        self.group_manage_selected = new.clamp(0, self.groups.len() as isize - 1) as usize;
    }

    pub(crate) fn handle_key_group_manage(&mut self, key: KeyEvent) -> Result<()> {
        self.group_notice = None;

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::Cancel, &key)
                || self.is_action(KeyAction::TabHosts, &key) =>
            {
                self.mode = AppMode::Normal;
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_group_manage_selection(1),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_group_manage_selection(-1),
            _ if self.is_action(KeyAction::AddHost, &key) => self.enter_group_form(None)?,
            _ if self.is_action(KeyAction::Edit, &key) => {
                if let Some(group) = self.groups.get(self.group_manage_selected).cloned() {
                    self.enter_group_form(Some(&group))?;
                }
            }
            _ if self.is_action(KeyAction::Delete, &key) => {
                if let Some(group) = self.groups.get(self.group_manage_selected).cloned() {
                    self.pending_delete = Some(PendingDelete::Group {
                        id: group.id,
                        name: group.name.clone(),
                    });
                    self.mode = AppMode::ConfirmDelete;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn enter_group_form(&mut self, existing: Option<&HostGroup>) -> Result<()> {
        // The default-identity selector reads `self.identities`; it is loaded
        // lazily elsewhere, so ensure it's populated before opening the form.
        // A seeded "Default" identity always exists, so empty means "not loaded".
        if self.identities.is_empty() {
            self.identities = self.store.list_identities()?;
        }
        let return_to_manage = self.mode == AppMode::GroupManage;
        let form = if let Some(group) = existing {
            GroupFormEdit {
                id: Some(group.id),
                name: group.name.clone(),
                cursor: text_input::char_len(&group.name),
                default_identity_id: group.default_identity_id,
                parent_id: group.parent_id,
                field: GroupFormField::Name,
                return_to_manage,
            }
        } else {
            GroupFormEdit {
                id: None,
                name: String::new(),
                cursor: 0,
                default_identity_id: None,
                parent_id: None,
                field: GroupFormField::Name,
                return_to_manage,
            }
        };
        self.group_form = Some(form);
        self.mode = AppMode::GroupForm;
        Ok(())
    }

    pub(crate) fn rename_selected_host_group(&mut self) -> Result<()> {
        let Some(group_id) = self.selected_host_group_id() else {
            self.host_notice = Some("Select a host in a group to rename it".into());
            return Ok(());
        };
        let Some(group) = self.groups.iter().find(|g| g.id == group_id).cloned() else {
            self.reload_hosts()?;
            return Ok(());
        };
        self.enter_group_form(Some(&group))
    }

    pub(crate) fn delete_selected_host_group(&mut self) -> Result<()> {
        let Some(group_id) = self.selected_host_group_id() else {
            self.host_notice = Some("Select a host in a group to delete it".into());
            return Ok(());
        };
        let name = self
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| "group".into());
        self.pending_delete = Some(PendingDelete::Group { id: group_id, name });
        self.mode = AppMode::ConfirmDelete;
        Ok(())
    }

    pub(crate) fn cancel_group_form(&mut self) -> Result<()> {
        let return_to_manage = self.group_form.as_ref().is_some_and(|f| f.return_to_manage);
        self.group_form = None;
        if return_to_manage {
            self.enter_group_manage()?;
        } else {
            self.mode = AppMode::Normal;
        }
        Ok(())
    }

    pub(crate) fn save_group_form(&mut self) -> Result<()> {
        let Some(form) = self.group_form.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        let name = form.name.trim();
        if name.is_empty() {
            self.host_notice = Some("Group name is required".into());
            self.group_form = Some(form);
            return Ok(());
        }

        if let Some(id) = form.id {
            self.store.update_group(
                id,
                &HostGroupUpdate {
                    name: Some(name.to_string()),
                    sort_order: None,
                    default_identity_id: Some(form.default_identity_id),
                    parent_id: Some(form.parent_id),
                },
            )?;
        } else {
            let sort_order = self.groups.len() as i32;
            self.store.create_group(&NewHostGroup {
                name: name.to_string(),
                sort_order,
                default_identity_id: form.default_identity_id,
                parent_id: form.parent_id,
            })?;
        }

        let return_to_manage = form.return_to_manage;
        self.reload_hosts()?;
        if return_to_manage {
            self.enter_group_manage()?;
        } else {
            self.mode = AppMode::Normal;
        }
        Ok(())
    }

    pub(crate) fn handle_key_group_form(&mut self, key: KeyEvent) -> Result<()> {
        if self.group_form.is_none() {
            return Ok(());
        }

        let field = self.group_form.as_ref().map(|f| f.field);
        match key.code {
            KeyCode::Esc => self.cancel_group_form()?,
            _ if self.is_save_key(&key) => self.save_group_form()?,
            // Move focus between fields.
            KeyCode::Up | KeyCode::BackTab => self.group_form_move_field(-1),
            KeyCode::Down | KeyCode::Tab => self.group_form_move_field(1),
            // Enter: save on the name field, open the dropdown on picker fields.
            KeyCode::Enter => match field {
                Some(GroupFormField::Name) => self.save_group_form()?,
                Some(kind) => self.open_group_field_picker(kind),
                None => {}
            },
            // Space also opens the dropdown on picker fields.
            KeyCode::Char(' ') if field != Some(GroupFormField::Name) => {
                if let Some(kind) = field {
                    self.open_group_field_picker(kind);
                }
            }
            KeyCode::Backspace
                if key.modifiers.is_empty() && field == Some(GroupFormField::Name) =>
            {
                self.group_form_backspace()
            }
            // Typing only edits the name field.
            KeyCode::Char(c)
                if field == Some(GroupFormField::Name)
                    && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.group_form_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    /// Move focus between the group-form fields, wrapping.
    pub(crate) fn group_form_move_field(&mut self, delta: i32) {
        let Some(form) = self.group_form.as_mut() else {
            return;
        };
        let all = GroupFormField::ALL;
        let cur = all.iter().position(|f| *f == form.field).unwrap_or(0) as i32;
        let next = (cur + delta).rem_euclid(all.len() as i32) as usize;
        form.field = all[next];
    }

    /// Open the dropdown list picker for the group form's Parent or Identity
    /// field, pre-selecting the current value.
    pub(crate) fn open_group_field_picker(&mut self, kind: GroupFormField) {
        if kind == GroupFormField::Name {
            return;
        }
        let Some(form) = self.group_form.as_ref() else {
            return;
        };
        let selected = match kind {
            GroupFormField::Parent => match form.parent_id {
                None => 0,
                Some(id) => self
                    .eligible_parents(form.id)
                    .iter()
                    .position(|&x| x == id)
                    .map_or(0, |p| p + 1),
            },
            GroupFormField::Identity => match form.default_identity_id {
                None => 0,
                Some(id) => self
                    .identities
                    .iter()
                    .position(|i| i.id == id)
                    .map_or(0, |p| p + 1),
            },
            GroupFormField::Name => 0,
        };
        self.group_field_picker = Some(GroupFieldPicker { kind, selected });
        self.mode = AppMode::GroupFieldPicker;
    }

    /// Rows shown in the group-form dropdown: the "(none)" slot first, then
    /// `(value id, label)` for each option.
    pub fn group_field_picker_options(&self) -> (String, Vec<(i64, String)>) {
        let Some(picker) = self.group_field_picker.as_ref() else {
            return ("(none)".into(), Vec::new());
        };
        match picker.kind {
            GroupFormField::Parent => {
                let self_id = self.group_form.as_ref().and_then(|f| f.id);
                let opts = self
                    .eligible_parents(self_id)
                    .into_iter()
                    .filter_map(|id| {
                        self.groups
                            .iter()
                            .find(|g| g.id == id)
                            .map(|g| (id, g.name.clone()))
                    })
                    .collect();
                ("(top level)".into(), opts)
            }
            GroupFormField::Identity => {
                let opts = self
                    .identities
                    .iter()
                    .map(|i| (i.id, i.name.clone()))
                    .collect();
                ("(none)".into(), opts)
            }
            GroupFormField::Name => ("(none)".into(), Vec::new()),
        }
    }

    pub(crate) fn handle_key_group_field_picker(&mut self, key: KeyEvent) -> Result<()> {
        let (_, options) = self.group_field_picker_options();
        let len = options.len() + 1; // +1 for the none slot
        match key.code {
            KeyCode::Esc => self.close_group_field_picker(),
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(p) = self.group_field_picker.as_mut() {
                    p.selected = (p.selected + 1) % len;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(p) = self.group_field_picker.as_mut() {
                    p.selected = (p.selected + len - 1) % len;
                }
            }
            KeyCode::Enter => {
                let (kind, sel) = self
                    .group_field_picker
                    .as_ref()
                    .map(|p| (p.kind, p.selected))
                    .unwrap_or((GroupFormField::Name, 0));
                let new_id = if sel == 0 {
                    None
                } else {
                    options.get(sel - 1).map(|(id, _)| *id)
                };
                if let Some(form) = self.group_form.as_mut() {
                    match kind {
                        GroupFormField::Parent => form.parent_id = new_id,
                        GroupFormField::Identity => form.default_identity_id = new_id,
                        GroupFormField::Name => {}
                    }
                }
                self.close_group_field_picker();
            }
            _ => {}
        }
        Ok(())
    }

    fn close_group_field_picker(&mut self) {
        self.group_field_picker = None;
        self.mode = AppMode::GroupForm;
    }

    pub(crate) fn eligible_parents(&self, group_id: Option<i64>) -> Vec<i64> {
        let banned = match group_id {
            Some(id) => {
                let mut set = self.group_descendants(id);
                set.insert(id);
                set
            }
            None => std::collections::HashSet::new(),
        };
        self.groups
            .iter()
            .filter(|g| !banned.contains(&g.id))
            .map(|g| g.id)
            .collect()
    }

    /// All transitive descendants of `group_id` (children, grandchildren, …).
    pub(crate) fn group_descendants(&self, group_id: i64) -> std::collections::HashSet<i64> {
        let mut out = std::collections::HashSet::new();
        let mut stack = vec![group_id];
        while let Some(cur) = stack.pop() {
            for g in self.groups.iter().filter(|g| g.parent_id == Some(cur)) {
                if out.insert(g.id) {
                    stack.push(g.id);
                }
            }
        }
        out
    }

    pub(crate) fn group_form_insert(&mut self, ch: char) {
        let Some(form) = self.group_form.as_mut() else {
            return;
        };
        form.cursor = text_input::insert_at(&mut form.name, form.cursor, ch);
    }

    pub(crate) fn group_form_backspace(&mut self) {
        let Some(form) = self.group_form.as_mut() else {
            return;
        };
        form.cursor = text_input::backspace_at(&mut form.name, form.cursor);
    }
}
