//! Workspaces (#73): a split-view layout saved under a name, reopened from
//! the picker. A workspace stores what to open in each pane, never live
//! sessions: saved hosts by name and local shells. Anything else (an ad-hoc
//! destination, a key push) comes back as an empty pane.

use serde::{Deserialize, Serialize};

use super::panes::{PaneTree, Panes};
use super::*;

/// What a saved pane opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedPane {
    Empty,
    Local,
    Host(String),
}

impl SavedPane {
    fn label(&self) -> &str {
        match self {
            Self::Empty => "empty",
            Self::Local => "local shell",
            Self::Host(name) => name,
        }
    }
}

impl App {
    fn saved_pane(&self, index: Option<usize>) -> SavedPane {
        let Some(session) = index.and_then(|i| self.sessions.get(i)) else {
            return SavedPane::Empty;
        };
        if session.local_shell {
            SavedPane::Local
        } else if session.key_push_identity.is_none()
            && self.hosts.iter().any(|h| h.name() == session.host_name)
        {
            SavedPane::Host(session.host_name.clone())
        } else {
            SavedPane::Empty
        }
    }

    /// The layout on screen (or last on screen, from the dashboard), as
    /// something that can be saved. `None` with no session to save.
    pub(crate) fn current_layout(&self) -> Option<PaneTree<SavedPane>> {
        match self.panes.as_ref() {
            Some(panes) => Some(
                panes
                    .tree
                    .map(&mut |id| self.saved_pane(id.and_then(|id| self.session_index(id)))),
            ),
            None => self
                .active_session
                .filter(|&i| i < self.sessions.len())
                .map(|i| PaneTree::Leaf(self.saved_pane(Some(i)))),
        }
    }

    pub(crate) fn open_workspace_picker(&mut self) {
        match self.store.list_workspaces() {
            Ok(list) => self.workspaces = list,
            Err(e) => {
                self.pane_notice(format!("workspaces unavailable: {e:#}"));
                return;
            }
        }
        self.open_session_picker(SessionPickerPurpose::Workspaces);
    }

    /// Picker rows: "save as <query>" first while there is something to save
    /// and a name typed, then the saved workspaces matching the query.
    pub(crate) fn picker_workspace_rows(&self, query: &str) -> Vec<PickerRow> {
        let name = query.trim();
        let mut rows = Vec::new();
        if !name.is_empty() && self.current_layout().is_some() {
            rows.push(PickerRow {
                index: self.workspaces.len(),
                badge: None,
                ordinal: None,
                name: format!("save this layout as \"{name}\""),
                endpoint: String::new(),
                current: false,
            });
        }
        let needle = name.to_lowercase();
        rows.extend(
            self.workspaces
                .iter()
                .enumerate()
                .filter(|(_, w)| w.name.to_lowercase().contains(&needle))
                .map(|(index, w)| PickerRow {
                    index,
                    badge: None,
                    ordinal: None,
                    name: w.name.clone(),
                    endpoint: serde_json::from_str::<PaneTree<SavedPane>>(&w.layout)
                        .map(|t| {
                            let leaves = t.leaves();
                            let names: Vec<&str> = leaves.iter().map(|p| p.label()).collect();
                            format!("{} panes: {}", names.len(), names.join(", "))
                        })
                        .unwrap_or_else(|_| "unreadable layout".into()),
                    current: false,
                }),
        );
        rows
    }

    /// Enter on a workspace row (`index` into `workspaces`, or one past the end
    /// for the save row). The picker is already closed.
    pub(crate) fn pick_workspace(&mut self, index: usize, query: &str) -> Result<()> {
        if index == self.workspaces.len() {
            let name = query.trim();
            let Some(layout) = self.current_layout() else {
                return Ok(());
            };
            let json = serde_json::to_string(&layout)?;
            let msg = match self.store.save_workspace(name, &json) {
                Ok(()) => format!("saved workspace \"{name}\""),
                Err(e) => format!("saving workspace failed: {e:#}"),
            };
            self.pane_notice(msg);
            return Ok(());
        }
        let Some(workspace) = self.workspaces.get(index) else {
            return Ok(());
        };
        match serde_json::from_str::<PaneTree<SavedPane>>(&workspace.layout) {
            Ok(layout) => self.open_workspace(layout),
            Err(e) => {
                self.pane_notice(format!(
                    "workspace \"{}\" is unreadable: {e}",
                    workspace.name
                ));
                Ok(())
            }
        }
    }

    /// Delete the workspace at `index` and refresh the list.
    pub(crate) fn delete_workspace_at(&mut self, index: usize) {
        let Some(id) = self.workspaces.get(index).map(|w| w.id) else {
            return;
        };
        if let Err(e) = self.store.delete_workspace(id) {
            self.pane_notice(format!("deleting workspace failed: {e:#}"));
        }
        self.workspaces = self.store.list_workspaces().unwrap_or_default();
    }

    /// Lay `layout` out and open each pane's session into it. The layout goes
    /// up first, so every session is spawned at its own pane's grid and never
    /// hears a SIGWINCH for it. Sessions already open stay in the strip.
    pub(crate) fn open_workspace(&mut self, layout: PaneTree<SavedPane>) -> Result<()> {
        let leaves: Vec<SavedPane> = layout.leaves().into_iter().cloned().collect();
        self.panes = (leaves.len() > 1).then(|| Panes {
            tree: layout.map(&mut |_| None),
            focused: 0,
            zoomed: false,
        });
        let mut missing = Vec::new();
        for (i, leaf) in leaves.iter().enumerate() {
            if let Some(panes) = self.panes.as_mut() {
                panes.focused = i;
            }
            let before = self.sessions.len();
            match leaf {
                SavedPane::Empty => {}
                SavedPane::Local => self.open_local_shell()?,
                SavedPane::Host(name) => {
                    match self.hosts.iter().find(|h| h.name() == name).cloned() {
                        Some(entry) => self.connect_host_entry(entry)?,
                        None => missing.push(name.clone()),
                    }
                }
            }
            if self.sessions.len() > before {
                self.show_in_focused_pane(self.sessions.len() - 1);
            }
        }
        if self.sessions.is_empty() {
            self.panes = None;
            self.active_session = None;
            self.mode = AppMode::Normal;
        } else {
            if let Some(panes) = self.panes.as_mut() {
                panes.focused = 0;
            }
            self.mode = AppMode::Session;
            self.follow_focused_pane();
        }
        if !missing.is_empty() {
            self.pane_notice(format!("not in the host list: {}", missing.join(", ")));
        }
        Ok(())
    }
}
