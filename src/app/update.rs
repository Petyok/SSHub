//! The update key and the release-check results (#149). The work itself runs
//! in [`crate::update`].

use super::*;
use crate::update::UpdateEvent;

/// Where the update flow stands, plus the worker answering it. Process-wide:
/// a profile switch carries it over.
#[derive(Default)]
pub struct UpdateState {
    pub status: UpdateStatus,
    rx: Option<Receiver<UpdateEvent>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    #[default]
    Idle,
    /// `manual`: asked for with the update key, so "up to date" and a failed
    /// check are reported too. The start-up check stays quiet about both.
    Checking {
        manual: bool,
    },
    /// A newer release tag (`v0.18.0`).
    Available(String),
    Installing(String),
    /// On disk; the running process is still the old version.
    Installed(String),
}

impl App {
    /// Ask GitHub for the latest release in the background.
    pub fn start_update_check(&mut self, manual: bool) {
        self.update.status = UpdateStatus::Checking { manual };
        self.update.rx = Some(crate::update::spawn_check());
    }

    /// The update key: install a known update, otherwise check for one.
    pub(crate) fn press_update_key(&mut self) {
        let notice = match self.update.status.clone() {
            UpdateStatus::Idle => {
                self.start_update_check(true);
                "Checking for updates…".to_string()
            }
            UpdateStatus::Checking { .. } => {
                // The start-up check is still out: report its answer now.
                self.update.status = UpdateStatus::Checking { manual: true };
                "Checking for updates…".to_string()
            }
            UpdateStatus::Available(tag) => {
                self.update.rx = Some(crate::update::spawn_install(tag.clone()));
                self.update.status = UpdateStatus::Installing(tag.clone());
                format!("Updating sshub to {tag}…")
            }
            UpdateStatus::Installing(tag) => format!("Update to {tag} in progress…"),
            UpdateStatus::Installed(tag) => format!("{tag} installed: restart sshub to use it"),
        };
        self.host_notice = Some(notice);
    }

    /// Whether an install is running: quitting now would abandon it.
    pub(crate) fn update_installing(&self) -> bool {
        matches!(self.update.status, UpdateStatus::Installing(_))
    }

    /// Apply the worker's answer, if it has one. Called every tick.
    pub fn poll_update(&mut self) {
        let Some(event) = self.update.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.update.rx = None;
        let key = self.config.keybinds.primary(KeyAction::Update).to_string();
        match (event, std::mem::take(&mut self.update.status)) {
            (UpdateEvent::Checked(result), UpdateStatus::Checking { manual }) => match result {
                Ok(Some(tag)) => {
                    let notice = if key.is_empty() {
                        format!("sshub {tag} is available")
                    } else {
                        format!("sshub {tag} is available: {key} to update")
                    };
                    // An answer to the key replaces "Checking…". The start-up
                    // check never joins a waiting warning (keyring, theme): the
                    // joined toast is too wide to draw, which would hide both,
                    // and the badge and footer hint announce the update anyway.
                    if manual || self.host_notice.is_none() {
                        self.host_notice = Some(notice);
                    }
                    self.update.status = UpdateStatus::Available(tag);
                }
                Ok(None) if manual => {
                    self.host_notice = Some(format!(
                        "sshub v{} is up to date",
                        env!("CARGO_PKG_VERSION")
                    ));
                }
                Err(err) if manual => self.report_update_failure(
                    format!("Update check failed:\n{err:#}"),
                    format!("Update check failed: {key} to retry"),
                ),
                _ => {}
            },
            (UpdateEvent::Installed(Ok(())), UpdateStatus::Installing(tag)) => {
                self.host_notice = Some(format!("{tag} installed: restart sshub to use it"));
                self.update.status = UpdateStatus::Installed(tag);
            }
            (UpdateEvent::Installed(Err(err)), UpdateStatus::Installing(tag)) => {
                self.report_update_failure(
                    format!("Update to {tag} failed:\n{err:#}"),
                    format!("Update to {tag} failed: {key} to retry"),
                );
                self.update.status = UpdateStatus::Available(tag);
            }
            (_, status) => self.update.status = status,
        }
    }

    /// The reason (with the command that failed) rarely fits the one-line
    /// toast, which is dropped when too wide, so it gets the popup when the
    /// dashboard is up; elsewhere a short toast waits for the dashboard.
    fn report_update_failure(&mut self, reason: String, short: String) {
        if self.mode == AppMode::Normal {
            self.notice_popup = Some(reason);
            self.mode = AppMode::Notice;
            self.host_notice = None;
        } else {
            self.host_notice = Some(short);
        }
    }

    /// Short state shown after the version in the tab bar, when there is any.
    pub fn update_badge(&self) -> Option<String> {
        match &self.update.status {
            UpdateStatus::Available(tag) => Some(format!("↑ {tag}")),
            UpdateStatus::Installing(tag) => Some(format!("updating to {tag}…")),
            UpdateStatus::Installed(tag) => Some(format!("restart for {tag}")),
            UpdateStatus::Idle | UpdateStatus::Checking { .. } => None,
        }
    }
}
