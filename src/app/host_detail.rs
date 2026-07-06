use super::*;

impl App {
    pub(crate) fn enter_host_detail(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };
        let tags = self.hosts[host_idx].tags().join(", ");
        let description = self.hosts[host_idx]
            .description()
            .unwrap_or_default()
            .to_string();
        let environment = self.hosts[host_idx]
            .environment()
            .unwrap_or_default()
            .to_string();
        self.detail_edit = Some(HostDetailEdit {
            tags: tags.clone(),
            description,
            environment,
            field: DetailEditField::Tags,
            cursor: text_input::char_len(&tags),
        });
        self.mode = AppMode::HostDetail;
        Ok(())
    }

    pub(crate) fn cancel_host_detail(&mut self) -> Result<()> {
        if let Some(host_idx) = self.selected_host_index() {
            let host_name = self.hosts[host_idx].name().to_string();
            if let Some((_, meta)) = self.hosts[host_idx].legacy_mut() {
                if let Some(stored) = self.metadata.get(&host_name)? {
                    *meta = stored;
                }
            }
        }
        self.detail_edit = None;
        self.mode = AppMode::Normal;
        Ok(())
    }

    pub(crate) fn save_host_detail(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            self.detail_edit = None;
            self.mode = AppMode::Normal;
            return Ok(());
        };
        let Some(edit) = self.detail_edit.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        let host_name = self.hosts[host_idx].name().to_string();
        let favorite = self.hosts[host_idx].favorite();
        let last_connected = self.hosts[host_idx].last_connected();
        let description = optional_field(&edit.description);
        let environment = optional_field(&edit.environment);
        let tags = parse_tags(&edit.tags);

        // Managed hosts (launcher + imported ssh_config rows) live in
        // launcher.db — persist there, or the edit is lost on reload.
        if let HostEntry::Managed(managed) = &self.hosts[host_idx] {
            let id = managed.id;
            let update = crate::store::HostUpdate {
                tags: Some(tags),
                notes: Some(description),
                environment: Some(environment),
                ..Default::default()
            };
            if let Some(updated) = self.store.update_host(id, &update)? {
                self.hosts[host_idx] = HostEntry::Managed(updated);
            }
        } else {
            let meta = crate::metadata::HostMetadata {
                host_name: host_name.clone(),
                tags,
                description,
                environment,
                favorite,
                last_connected,
            };
            self.metadata.upsert(&meta)?;
            if let Some((_, stored_meta)) = self.hosts[host_idx].legacy_mut() {
                *stored_meta = meta;
            }
        }
        self.rebuild_filter();
        self.mode = AppMode::Normal;
        Ok(())
    }

    pub(crate) fn detail_edit_field_next(&mut self) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        edit.field = edit.field.next();
        edit.cursor = text_input::char_len(edit.active_field());
    }

    pub(crate) fn detail_edit_field_prev(&mut self) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        edit.field = edit.field.prev();
        edit.cursor = text_input::char_len(edit.active_field());
    }

    pub(crate) fn detail_edit_backspace(&mut self) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        let c = edit.cursor;
        edit.cursor = text_input::backspace_at(edit.active_field_mut(), c);
    }

    pub(crate) fn detail_edit_insert(&mut self, ch: char) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        let c = edit.cursor;
        edit.cursor = text_input::insert_at(edit.active_field_mut(), c, ch);
    }
}
