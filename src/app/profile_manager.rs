use super::*;
use crate::profile::picker::{PickerOutcome, ProfilePicker};
use crate::profile::{ProfileState, RootDirs};

impl App {
    fn profile_roots(&self) -> Option<RootDirs> {
        self.profile
            .as_ref()
            .filter(|p| !p.compat)
            .map(|p| RootDirs {
                data_root: p.data_root.clone(),
                config_root: p.data_root.clone(),
                compat: false,
            })
    }

    pub(crate) fn refresh_profile_count(&mut self) {
        if let Some(roots) = self.profile_roots() {
            if let Ok(Some(state)) = ProfileState::load(&roots.data_root) {
                self.profile_count = state.profiles.len();
            }
        }
    }

    pub(crate) fn profile_action_label(&self) -> &'static str {
        if self.profile_count <= 1 {
            "Add profile"
        } else {
            "Manage profiles"
        }
    }

    pub(crate) fn active_profile_name(&self) -> &str {
        self.profile
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("default")
    }

    /// Refuse to discard work whose lifetime or pending audit belongs to this profile.
    pub fn profile_switch_blocker(&self) -> Option<&'static str> {
        if self.sessions.iter().any(|s| !s.phase.is_terminal()) {
            return Some("Close SSH and local shell sessions before switching profiles.");
        }
        if self.sftp.is_some()
            || self.sftp_tx.is_some()
            || self.sftp_rx.is_some()
            || self.sftp_tx2.is_some()
            || self.sftp_rx2.is_some()
            || self.sftp_relay.is_some()
        {
            return Some("Disconnect SFTP and finish transfers before switching profiles.");
        }
        if self.broadcast.is_some() || self.broadcast_setup.is_some() {
            return Some("Finish and dismiss the broadcast before switching profiles.");
        }
        // Includes children still proving and scheduled reconnects, even if the
        // cached tunnel list no longer contains their record.
        if self.tunnel_manager.has_active_work() {
            return Some("Stop tunnels and pending reconnects before switching profiles.");
        }
        None
    }

    pub fn open_profile_manager(&mut self) {
        let Some(roots) = self.profile_roots() else {
            self.host_notice = Some(
                "Profile management is unavailable in compatibility mode (directory overrides)."
                    .into(),
            );
            self.host_notice_at = Some(std::time::Instant::now());
            return;
        };
        let state = match ProfileState::load(&roots.data_root) {
            Ok(Some(state)) if !state.profiles.is_empty() => state,
            Ok(_) => {
                self.host_notice = Some(
                    "Profile registry is missing or empty; cannot open profile management.".into(),
                );
                self.host_notice_at = Some(std::time::Instant::now());
                return;
            }
            Err(error) => {
                self.host_notice = Some(format!("Cannot open profiles: {error}"));
                self.host_notice_at = Some(std::time::Instant::now());
                return;
            }
        };
        let active_id = self.profile.as_ref().unwrap().id.clone();
        if state.by_id(&active_id).is_none() {
            self.host_notice =
                Some("Active profile is missing from the registry; cannot manage profiles.".into());
            self.host_notice_at = Some(std::time::Instant::now());
            return;
        }
        self.profile_count = state.profiles.len();
        self.profile_return_mode = self.mode;
        self.pending_profile = None;
        self.profile_picker = Some(ProfilePicker::in_app(roots, state, active_id));
        self.mode = AppMode::ProfilePicker;
    }

    fn close_profile_manager(&mut self) {
        self.profile_picker = None;
        self.pending_profile = None;
        self.mode = self.profile_return_mode;
    }

    pub fn handle_key_profile_picker(&mut self, key: KeyEvent) -> Result<()> {
        let Some(picker) = self.profile_picker.as_mut() else {
            self.mode = self.profile_return_mode;
            return Ok(());
        };
        let outcome = picker.handle_key(key)?;
        self.profile_count = picker.profile_count();
        match outcome {
            PickerOutcome::Continue => {}
            PickerOutcome::Quit => self.close_profile_manager(),
            PickerOutcome::Launch(record) => {
                if self.profile.as_ref().is_some_and(|p| p.id == record.id) {
                    self.close_profile_manager();
                    return Ok(());
                }
                if let Some(message) = self.profile_switch_blocker() {
                    self.profile_picker
                        .as_mut()
                        .unwrap()
                        .set_error(message.into());
                    return Ok(());
                }
                let roots = self
                    .profile_roots()
                    .expect("manager requires profile roots");
                let paths = (|| {
                    crate::profile::require_profile_dir(&roots, &record)?;
                    let ssh_config = crate::profile::ssh_config_path_for_profile(&roots, &record)?;
                    Ok::<_, anyhow::Error>(crate::profile::profile_paths(
                        &roots, &record, ssh_config,
                    ))
                })();
                match paths {
                    Ok(paths) => self.pending_profile = Some(paths),
                    Err(error) => self
                        .profile_picker
                        .as_mut()
                        .unwrap()
                        .set_error(format!("Cannot switch profiles: {error}")),
                }
            }
        }
        Ok(())
    }
}
