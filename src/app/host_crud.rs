use super::*;

impl App {
    pub(crate) fn edit_selected_host(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };
        let managed = self.hosts[host_idx].managed().cloned();
        match managed.as_ref().map(|m| m.source) {
            Some(HostSource::SshConfig) => {
                self.enter_host_form(managed.as_ref(), true)?;
            }
            Some(HostSource::Launcher) => {
                self.enter_host_form(managed.as_ref(), false)?;
            }
            None => {
                // Legacy ssh_config alias with no launcher row yet: materialize
                // it into launcher.db so it gains a group/identity/metadata
                // overlay, then edit that full form instead of the tags-only
                // HostDetail (which has no Group field).
                let name = self.hosts[host_idx].name().to_string();
                let materialized = crate::ssh::materialize_ssh_config_host(
                    self.resolver.as_ref(),
                    &self.store,
                    self.metadata.as_ref(),
                    &name,
                )?;
                if materialized {
                    self.reload_hosts()?;
                    self.restore_selection_by_name(&name);
                    let managed = self
                        .selected_host_index()
                        .and_then(|idx| self.hosts[idx].managed().cloned());
                    if managed.is_some() {
                        self.enter_host_form(managed.as_ref(), true)?;
                        return Ok(());
                    }
                }
                self.enter_host_detail()?;
            }
        }
        Ok(())
    }

    pub(crate) fn delete_selected_host(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };
        let Some(id) = self.hosts[host_idx].managed_id() else {
            self.host_notice = Some("Only launcher hosts can be deleted".into());
            return Ok(());
        };
        if self.hosts[host_idx].source() != HostSource::Launcher {
            self.host_notice = Some("Only launcher hosts can be deleted".into());
            return Ok(());
        }
        let name = self.hosts[host_idx].display_name().to_string();
        self.pending_delete = Some(PendingDelete::Host { id, name });
        self.mode = AppMode::ConfirmDelete;
        Ok(())
    }

    pub(crate) fn duplicate_selected_host(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };

        let copy_name = match &self.hosts[host_idx] {
            HostEntry::Managed(m) => {
                let Some(copy) = self.store.duplicate_host(m.id)? else {
                    self.host_notice = Some("Host not found".into());
                    return Ok(());
                };
                copy.name
            }
            HostEntry::Legacy { host, meta } => self.duplicate_legacy_to_launcher(host, meta)?,
        };

        self.reload_hosts()?;
        self.restore_selection_by_name(&copy_name);
        Ok(())
    }

    pub(crate) fn duplicate_legacy_to_launcher(
        &self,
        host: &SshHost,
        meta: &crate::metadata::HostMetadata,
    ) -> Result<String> {
        let mut name = format!("{}-copy", host.name);
        let mut suffix = 2u32;
        while self.store.get_host_by_name(&name)?.is_some() {
            name = format!("{}-copy-{}", host.name, suffix);
            suffix += 1;
        }

        let address = host.hostname.clone().unwrap_or_else(|| host.name.clone());
        let port = host.port.unwrap_or(22);

        let mut new_host = NewHost::launcher(name.clone(), address);
        new_host.port = port;
        new_host.tags = meta.tags.clone();
        new_host.notes = meta.description.clone();
        new_host.proxy_jump = host.proxy_jump.clone();
        new_host.forward_agent = host.forward_agent.unwrap_or(false);
        new_host.remote_command = host.remote_command.clone();
        new_host.session_logging = meta.session_logging;
        new_host.identity_id = self.match_identity_for_ssh_host(host)?;
        self.store.create_host(&new_host)?;
        Ok(name)
    }

    pub(crate) fn match_identity_for_ssh_host(&self, host: &SshHost) -> Result<Option<i64>> {
        let user = host.user.as_deref();
        let key = host.identity_file.as_deref();
        if user.is_none() && key.is_none() {
            return Ok(None);
        }

        for identity in self.store.list_identities()? {
            let id_user = identity.username.as_deref();
            let id_key = identity
                .private_key
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned());
            let matches = match (user, key) {
                (Some(u), Some(k)) => id_user == Some(u) && id_key.as_deref() == Some(k),
                (Some(u), None) => id_user == Some(u),
                (None, Some(k)) => id_key.as_deref() == Some(k),
                (None, None) => false,
            };
            if matches {
                return Ok(Some(identity.id));
            }
        }

        let mut identity_name = format!("{}-identity", host.name);
        let mut suffix = 2u32;
        while self.store.get_identity_by_name(&identity_name)?.is_some() {
            identity_name = format!("{}-identity-{}", host.name, suffix);
            suffix += 1;
        }

        let created = self.store.create_identity(&NewIdentity {
            name: identity_name,
            username: host.user.clone(),
            private_key: key.map(PathBuf::from),
            ..Default::default()
        })?;
        Ok(Some(created.id))
    }
}
