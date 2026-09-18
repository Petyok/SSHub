use super::profile_transfer::{needs_explicit_confirm, TransferScope};
use super::*;
use crate::profile::picker::{PickerOutcome, ProfilePicker};
use crate::profile::{ProfileState, RootDirs};
use anyhow::Context;

impl App {
    pub(crate) fn profile_roots(&self) -> Option<RootDirs> {
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
        self.pending_transfer = None;
        self.profile_picker = Some(ProfilePicker::in_app(roots, state, active_id));
        self.mode = AppMode::ProfilePicker;
    }

    fn close_profile_manager(&mut self) {
        self.profile_picker = None;
        self.pending_profile = None;
        self.pending_transfer = None;
        self.mode = self.profile_return_mode;
    }

    pub fn handle_key_profile_picker(&mut self, key: KeyEvent) -> Result<()> {
        // An armed cross-profile transfer owns every key until it is confirmed
        // (`y`) or cancelled (`Esc`), so typing a destination name never leaks
        // into the picker.
        if self.pending_transfer.is_some() {
            return self.handle_key_profile_transfer(key);
        }
        // Manager-level single-key actions fire from the list view only: while
        // creating or renaming a profile the same keystroke is editor text.
        let picker_list_view = self
            .profile_picker
            .as_ref()
            .is_some_and(|picker| picker.is_list_view());
        let Some(picker) = self.profile_picker.as_mut() else {
            self.mode = self.profile_return_mode;
            return Ok(());
        };
        let outcome = picker.handle_key(key)?;
        // `t`/`T` opens the transfer flow. The highlighted profile is the
        // picker cursor, which this side cannot read, so the destination name
        // is typed explicitly in the next step.
        let arm_transfer = matches!(key.code, KeyCode::Char('t') | KeyCode::Char('T'))
            && matches!(outcome, PickerOutcome::Continue)
            && picker_list_view;
        if arm_transfer {
            self.arm_profile_transfer();
            if let Some(picker) = self.profile_picker.as_mut() {
                picker.clear_error();
            }
            return Ok(());
        }
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

    /// Clear the dialog's transient error line; the staged flow renders in its
    /// own popup, so there is no picker message to re-show.
    pub(crate) fn show_transfer_status(&mut self) {
        if let Some(pending) = self.pending_transfer.as_mut() {
            pending.notice = None;
        }
    }

    fn show_transfer_error(&mut self, error: anyhow::Error) {
        let message = format!("{error:#}");
        if let Some(pending) = self.pending_transfer.as_mut() {
            pending.notice = Some(message);
        } else if let Some(picker) = self.profile_picker.as_mut() {
            picker.set_error(message);
        } else {
            self.host_notice = Some(message);
        }
    }

    /// Staged transfer keys. Destination → scope → (group-only vs with-hosts)
    /// → plan → explicit `y` confirm. Only `y` with a built plan applies;
    /// everything else adjusts or cancels.
    pub(crate) fn handle_key_profile_transfer(&mut self, key: KeyEvent) -> Result<()> {
        if matches!(key.code, KeyCode::Esc) {
            self.cancel_pending_transfer();
            if let Some(picker) = self.profile_picker.as_mut() {
                picker.set_error("transfer cancelled".into());
            }
            return Ok(());
        }
        let dest_missing = self
            .pending_transfer
            .as_ref()
            .is_some_and(|pending| pending.dest_name.is_none());
        if dest_missing {
            // Fuzzy destination picker: typing filters the candidate profiles
            // (same nucleo wiring as the host search), ↑↓ move the highlight,
            // Enter validates the highlighted name through
            // `choose_transfer_dest`, Esc cancels the whole flow above.
            match key.code {
                KeyCode::Enter => {
                    let choice = self
                        .pending_transfer
                        .as_ref()
                        .and_then(|pending| pending.dest_highlight())
                        .map(str::to_string);
                    match choice {
                        Some(name) => {
                            if let Err(error) = self.choose_transfer_dest(&name) {
                                self.show_transfer_error(error);
                            } else {
                                self.show_transfer_status();
                            }
                        }
                        None => {
                            if let Some(pending) = self.pending_transfer.as_mut() {
                                pending.notice = Some(
                                    "no matching profile — keep typing or Esc to cancel".into(),
                                );
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Some(pending) = self.pending_transfer.as_mut() {
                        pending.dest_buffer.pop();
                        pending.refresh_dest_filter();
                    }
                    self.show_transfer_status();
                }
                KeyCode::Up => {
                    if let Some(pending) = self.pending_transfer.as_mut() {
                        pending.move_dest_selection(-1);
                    }
                }
                KeyCode::Down => {
                    if let Some(pending) = self.pending_transfer.as_mut() {
                        pending.move_dest_selection(1);
                    }
                }
                KeyCode::Char(c) if !c.is_control() => {
                    if let Some(pending) = self.pending_transfer.as_mut() {
                        pending.dest_buffer.push(c);
                        pending.refresh_dest_filter();
                    }
                    self.show_transfer_status();
                }
                _ => self.show_transfer_status(),
            }
            return Ok(());
        }
        let scope_missing = self
            .pending_transfer
            .as_ref()
            .is_some_and(|pending| pending.scope.is_none());
        if scope_missing {
            let scope = match key.code {
                KeyCode::Char('h') => self
                    .transfer_host_name()
                    .context("no host selected on the dashboard")
                    .map(TransferScope::Host),
                KeyCode::Char('g') => self
                    .transfer_group_name()
                    .context("no group under cursor (open Groups first)")
                    .map(|name| TransferScope::Group {
                        name,
                        with_hosts: false,
                    }),
                KeyCode::Char('i') => self
                    .transfer_identity_name()
                    .context("no identity selected")
                    .map(TransferScope::Identity),
                KeyCode::Char('t') => self
                    .transfer_tunnel_name()
                    .context("no tunnel selected")
                    .map(TransferScope::Tunnel),
                _ => {
                    self.show_transfer_status();
                    return Ok(());
                }
            };
            match scope {
                Ok(scope) => {
                    if let Err(error) = self.choose_transfer_scope(scope) {
                        self.show_transfer_error(error);
                    } else {
                        self.show_transfer_status();
                    }
                }
                Err(error) => self.show_transfer_error(error),
            }
            return Ok(());
        }
        let group_choice_open = self.pending_transfer.as_ref().is_some_and(|pending| {
            matches!(pending.scope, Some(TransferScope::Group { .. })) && pending.plan.is_none()
        });
        if group_choice_open {
            match key.code {
                KeyCode::Char('g') => {
                    if let Err(error) = self.set_transfer_with_hosts(false) {
                        self.show_transfer_error(error);
                    } else {
                        self.show_transfer_status();
                    }
                }
                KeyCode::Char('w') => {
                    if let Err(error) = self.set_transfer_with_hosts(true) {
                        self.show_transfer_error(error);
                    } else {
                        self.show_transfer_status();
                    }
                }
                _ => self.show_transfer_status(),
            }
            return Ok(());
        }
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Explicit confirmation only: `needs_explicit_confirm` pins the
                // invariant that a plan applies solely via this key. An empty
                // plan (nothing runnable) refuses instead: confirming zero
                // items would silently "succeed" while transferring nothing.
                let empty_plan = self.pending_transfer.as_ref().is_some_and(|pending| {
                    pending
                        .plan
                        .as_ref()
                        .is_some_and(|plan| plan.runnable().is_empty())
                });
                if empty_plan {
                    if let Some(pending) = self.pending_transfer.as_mut() {
                        pending.notice =
                            Some("nothing to transfer — selection matched no items".into());
                    }
                } else {
                    let confirmed = self
                        .pending_transfer
                        .as_ref()
                        .and_then(|pending| pending.plan.as_ref())
                        .is_some_and(needs_explicit_confirm);
                    if !confirmed {
                        self.show_transfer_status();
                    } else if let Err(error) = self.apply_pending_transfer() {
                        self.show_transfer_error(error);
                    } else {
                        self.close_profile_manager();
                    }
                }
            }
            KeyCode::Char('m') => {
                if let Err(error) = self.toggle_transfer_move() {
                    self.show_transfer_error(error);
                } else {
                    self.show_transfer_status();
                }
            }
            KeyCode::Char('w') => {
                let is_group = self.pending_transfer.as_ref().is_some_and(|pending| {
                    matches!(pending.scope, Some(TransferScope::Group { .. }))
                });
                if is_group {
                    let next = self.pending_transfer.as_ref().is_some_and(|pending| {
                        matches!(
                            pending.scope,
                            Some(TransferScope::Group {
                                with_hosts: false,
                                ..
                            })
                        )
                    });
                    if let Err(error) = self.set_transfer_with_hosts(next) {
                        self.show_transfer_error(error);
                    } else {
                        self.show_transfer_status();
                    }
                } else {
                    self.show_transfer_status();
                }
            }
            _ => self.show_transfer_status(),
        }
        Ok(())
    }
}
