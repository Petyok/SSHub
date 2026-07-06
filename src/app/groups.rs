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
            KeyCode::Esc | KeyCode::Char('h') if key.modifiers.is_empty() => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('1') if key.modifiers.is_empty() => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down if key.modifiers.is_empty() => {
                self.move_group_manage_selection(1)
            }
            KeyCode::Char('k') | KeyCode::Up if key.modifiers.is_empty() => {
                self.move_group_manage_selection(-1)
            }
            KeyCode::Char('a') if key.modifiers.is_empty() => {
                self.enter_group_form(None)?;
            }
            KeyCode::Char('e') if key.modifiers.is_empty() => {
                if let Some(group) = self.groups.get(self.group_manage_selected).cloned() {
                    self.enter_group_form(Some(&group))?;
                }
            }
            KeyCode::Char('d') if key.modifiers.is_empty() => {
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
                return_to_manage,
            }
        } else {
            GroupFormEdit {
                id: None,
                name: String::new(),
                cursor: 0,
                default_identity_id: None,
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

    /// Open the dedicated default-identity picker for the selected group header.
    pub(crate) fn open_group_identity_picker(&mut self) -> Result<()> {
        let Some(group) = self
            .selected_nav_header()
            .and_then(|si| self.group_sections.get(si))
            .and_then(|section| section.group.clone())
        else {
            self.host_notice = Some("Ungrouped hosts have no default identity.".into());
            return Ok(());
        };
        if self.identities.is_empty() {
            self.identities = self.store.list_identities()?;
        }
        let selected = group
            .default_identity_id
            .and_then(|id| self.identities.iter().position(|i| i.id == id).map(|p| p + 1))
            .unwrap_or(0);
        self.group_identity_picker = Some(GroupIdentityPicker {
            group_id: group.id,
            group_name: group.name.clone(),
            selected,
        });
        self.mode = AppMode::GroupIdentityPicker;
        Ok(())
    }

    pub(crate) fn handle_key_group_identity_picker(&mut self, key: KeyEvent) -> Result<()> {
        // Ring of options: index 0 = "(none)", then one per identity.
        let len = self.identities.len() + 1;
        match key.code {
            KeyCode::Esc => {
                self.group_identity_picker = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Right => {
                if let Some(p) = self.group_identity_picker.as_mut() {
                    p.selected = (p.selected + 1) % len;
                }
            }
            KeyCode::Char('k') | KeyCode::Up | KeyCode::Left => {
                if let Some(p) = self.group_identity_picker.as_mut() {
                    p.selected = (p.selected + len - 1) % len;
                }
            }
            KeyCode::Enter => self.save_group_identity_picker()?,
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn save_group_identity_picker(&mut self) -> Result<()> {
        let Some(picker) = self.group_identity_picker.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };
        let new_id = if picker.selected == 0 {
            None
        } else {
            self.identities.get(picker.selected - 1).map(|i| i.id)
        };
        self.store.update_group(
            picker.group_id,
            &HostGroupUpdate {
                default_identity_id: Some(new_id),
                ..Default::default()
            },
        )?;
        self.reload_hosts()?;
        let name = self
            .identities
            .iter()
            .find(|i| Some(i.id) == new_id)
            .map(|i| i.name.clone());
        self.host_notice = Some(match name {
            Some(n) => format!("'{}' default identity → {n}", picker.group_name),
            None => format!("'{}' default identity cleared", picker.group_name),
        });
        self.mode = AppMode::Normal;
        Ok(())
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
                },
            )?;
        } else {
            let sort_order = self.groups.len() as i32;
            self.store.create_group(&NewHostGroup {
                name: name.to_string(),
                sort_order,
                default_identity_id: form.default_identity_id,
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

        match key.code {
            KeyCode::Esc => self.cancel_group_form()?,
            KeyCode::Enter => self.save_group_form()?,
            _ if self.is_save_key(&key) => self.save_group_form()?,
            KeyCode::Left => self.group_form_cycle_identity(-1),
            KeyCode::Right => self.group_form_cycle_identity(1),
            KeyCode::Backspace if key.modifiers.is_empty() => self.group_form_backspace(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.group_form_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    /// Cycle the group's default identity through `[none, id0, id1, …]`.
    pub(crate) fn group_form_cycle_identity(&mut self, delta: i32) {
        // Build the option ring: index 0 is "none", then each identity.
        let ids: Vec<i64> = self.identities.iter().map(|i| i.id).collect();
        let len = ids.len() as i32 + 1;
        let Some(form) = self.group_form.as_mut() else {
            return;
        };
        let cur = match form.default_identity_id {
            None => 0,
            Some(id) => ids.iter().position(|&x| x == id).map_or(0, |p| p as i32 + 1),
        };
        let next = (cur + delta).rem_euclid(len);
        form.default_identity_id = if next == 0 {
            None
        } else {
            Some(ids[(next - 1) as usize])
        };
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
