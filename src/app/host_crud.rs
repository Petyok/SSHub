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
            HostEntry::Legacy { host, meta } => {
                crate::hosts::duplicate_legacy_to_launcher(&self.store, host, meta)?
            }
        };

        self.reload_hosts()?;
        self.restore_selection_by_name(&copy_name);
        Ok(())
    }
}
