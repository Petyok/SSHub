//! SFTP-style host transfer: the dual-pane screen behind `m` in the profile
//! manager's transfer dialog.
//!
//! Left pane = the active profile's hosts, right pane = the destination
//! profile's hosts. `→` stages the left selection for a push, `←` stages the
//! right selection for a pull, a queue strip shows what is staged, `c`
//! continues. The staged hosts then go through the guided checklists
//! ([`super::profile_migration`]) one direction at a time, push first, each
//! ending in the usual plan review and explicit `y`.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::profile_migration::MigrationWizard;
use crate::store::transfer::TransferSnapshot;
use crate::store::LauncherStore;

/// Group label of hosts without a (real) primary group.
pub const NO_GROUP: &str = "(no group)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// One host of a pane: what the list shows and the filter searches.
#[derive(Clone, Debug)]
pub struct HostLine {
    pub name: String,
    pub address: String,
    pub group: String,
}

/// A visible pane row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneRow {
    Group {
        name: String,
        count: usize,
    },
    /// Index into [`Pane::hosts`].
    Host(usize),
}

/// One profile's hosts, grouped, with a filter and a cursor.
pub struct Pane {
    pub profile: String,
    pub hosts: Vec<HostLine>,
    pub filter: String,
    pub searching: bool,
    /// Cursor into [`Pane::rows`].
    pub selected: usize,
}

impl Pane {
    pub fn new(profile: String, snapshot: &TransferSnapshot) -> Self {
        let hosts = snapshot
            .hosts
            .iter()
            .map(|host| HostLine {
                name: host.name.clone(),
                address: host.address.clone(),
                group: host
                    .group
                    .as_ref()
                    .filter(|g| !g.reserved)
                    .map(|g| g.name.clone())
                    .unwrap_or_else(|| NO_GROUP.to_string()),
            })
            .collect();
        Self {
            profile,
            hosts,
            filter: String::new(),
            searching: false,
            selected: 0,
        }
    }

    fn matches(&self, host: &HostLine) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let needle = self.filter.to_lowercase();
        [&host.name, &host.address, &host.group]
            .iter()
            .any(|field| field.to_lowercase().contains(&needle))
    }

    /// Group headers (alphabetical, "(no group)" last), each followed by its
    /// matching hosts (alphabetical). Groups with no match are left out.
    pub fn rows(&self) -> Vec<PaneRow> {
        let mut groups: Vec<&str> = self.hosts.iter().map(|h| h.group.as_str()).collect();
        groups.sort_by(|a, b| {
            (*a == NO_GROUP)
                .cmp(&(*b == NO_GROUP))
                .then_with(|| a.cmp(b))
        });
        groups.dedup();
        let mut rows = Vec::new();
        for group in groups {
            let mut members: Vec<usize> = (0..self.hosts.len())
                .filter(|&i| self.hosts[i].group == group && self.matches(&self.hosts[i]))
                .collect();
            if members.is_empty() {
                continue;
            }
            members.sort_by(|&a, &b| self.hosts[a].name.cmp(&self.hosts[b].name));
            rows.push(PaneRow::Group {
                name: group.to_string(),
                count: members.len(),
            });
            rows.extend(members.into_iter().map(PaneRow::Host));
        }
        rows
    }

    /// Number of hosts the filter lets through.
    pub fn shown(&self) -> usize {
        self.hosts.iter().filter(|h| self.matches(h)).count()
    }

    pub fn move_by(&mut self, delta: isize) {
        let len = self.rows().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    /// Hosts under the cursor: the host itself, or every shown host of a group.
    pub fn selected_hosts(&self) -> Vec<String> {
        let rows = self.rows();
        match rows.get(self.selected) {
            Some(PaneRow::Host(i)) => vec![self.hosts[*i].name.clone()],
            Some(PaneRow::Group { name, .. }) => rows
                .iter()
                .filter_map(|row| match row {
                    PaneRow::Host(i) if &self.hosts[*i].group == name => {
                        Some(self.hosts[*i].name.clone())
                    }
                    _ => None,
                })
                .collect(),
            None => Vec::new(),
        }
    }

    /// Every host the filter lets through.
    pub fn shown_hosts(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .hosts
            .iter()
            .filter(|h| self.matches(h))
            .map(|h| h.name.clone())
            .collect();
        names.sort();
        names
    }
}

/// A staged host: pushed from the active profile, or pulled from the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    pub pull: bool,
    pub host: String,
}

/// The staged hosts of one direction, transferred as one guided migration.
#[derive(Clone, Debug)]
pub struct Batch {
    pub pull: bool,
    pub hosts: Vec<String>,
}

/// Split a queue into its push batch and its pull batch (push first), keeping
/// staging order and dropping empty batches.
pub fn batches_of(queue: &[Staged]) -> Vec<Batch> {
    [false, true]
        .into_iter()
        .map(|pull| Batch {
            pull,
            hosts: queue
                .iter()
                .filter(|s| s.pull == pull)
                .map(|s| s.host.clone())
                .collect(),
        })
        .filter(|batch| !batch.hosts.is_empty())
        .collect()
}

pub struct HostTransferScreen {
    pub left: Pane,
    pub right: Pane,
    pub focus: Side,
    pub queue: Vec<Staged>,
    pub notice: Option<String>,
}

impl HostTransferScreen {
    pub fn pane(&self, side: Side) -> &Pane {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    fn pane_mut(&mut self, side: Side) -> &mut Pane {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    pub fn is_staged(&self, side: Side, host: &str) -> bool {
        // Left hosts are staged by a push, right hosts by a pull.
        let pull = side == Side::Right;
        self.queue.iter().any(|s| s.pull == pull && s.host == host)
    }

    /// Whether the focused pane is taking search text (Esc then clears it
    /// instead of cancelling the whole transfer).
    pub fn searching(&self) -> bool {
        self.pane(self.focus).searching
    }

    /// Stage `hosts` from the focused pane toward the other one: a push from
    /// the left, a pull from the right.
    fn stage(&mut self, hosts: Vec<String>) {
        let pull = self.focus == Side::Right;
        let mut added = 0;
        for host in hosts {
            if !self.queue.iter().any(|s| s.pull == pull && s.host == host) {
                self.queue.push(Staged { pull, host });
                added += 1;
            }
        }
        self.notice = Some(if added == 0 {
            "nothing new to stage".to_string()
        } else {
            format!(
                "staged {added} host(s) for a {}",
                if pull { "pull" } else { "push" }
            )
        });
    }
}

impl super::profile_transfer::PendingTransfer {
    /// Whether `Esc` belongs to the dual-pane screen (it is taking search text)
    /// rather than to the whole transfer flow.
    pub(crate) fn screen_wants_escape(&self) -> bool {
        self.screen.as_ref().is_some_and(|s| s.searching())
    }
}

impl crate::app::App {
    /// Open the dual-pane screen over the active profile and the chosen
    /// destination, keeping `staged` (empty on a fresh start).
    pub(crate) fn open_host_transfer_screen(&mut self, staged: Vec<Staged>) -> Result<()> {
        let (other, other_db) = {
            let pending = self
                .pending_transfer
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no transfer in progress"))?;
            (
                pending.dest_name.clone().unwrap_or_default(),
                pending
                    .dest_db
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("choose a destination profile first"))?,
            )
        };
        let left = Pane::new(
            self.active_profile_name().to_string(),
            &TransferSnapshot::capture(&self.store)?,
        );
        let other_store = LauncherStore::open(&other_db)
            .map_err(|e| anyhow::anyhow!("cannot open destination profile database: {e:#}"))?;
        let right = Pane::new(other, &TransferSnapshot::capture(&other_store)?);
        anyhow::ensure!(
            !left.hosts.is_empty() || !right.hosts.is_empty(),
            "neither profile has any hosts"
        );
        if let Some(pending) = self.pending_transfer.as_mut() {
            pending.wizard = None;
            pending.scope = None;
            pending.plan = None;
            pending.dialog = None;
            pending.batches.clear();
            pending.screen = Some(HostTransferScreen {
                left,
                right,
                focus: Side::Left,
                queue: staged,
                notice: None,
            });
        }
        Ok(())
    }

    /// Start one batch: its snapshot comes from the profile the hosts leave,
    /// the hosts are pre-checked, and the wizard skips straight to the first
    /// dependent step (or to the plan review when there is none).
    pub(crate) fn start_batch(&mut self, batch: Batch) -> Result<()> {
        let other_db = self
            .pending_transfer
            .as_ref()
            .and_then(|p| p.dest_db.clone())
            .ok_or_else(|| anyhow::anyhow!("choose a destination profile first"))?;
        let (snapshot, counts) = if batch.pull {
            let other = LauncherStore::open(&other_db)
                .map_err(|e| anyhow::anyhow!("cannot open the other profile's database: {e:#}"))?;
            (
                TransferSnapshot::capture(&other)?,
                other.auth_event_counts()?,
            )
        } else {
            (
                TransferSnapshot::capture(&self.store)?,
                self.store.auth_event_counts()?,
            )
        };
        let mut wizard = MigrationWizard::new(snapshot, counts);
        wizard.preselect(&batch.hosts);
        let advanced = wizard.advance().map_err(|m| anyhow::anyhow!("{m}"))?;
        if let Some(pending) = self.pending_transfer.as_mut() {
            pending.pull = batch.pull;
            pending.scope = None;
            pending.plan = None;
            pending.dialog = None;
            pending.wizard = Some(wizard);
        }
        if advanced == super::profile_migration::Advance::Review {
            if let Some(error) = self.finish_migration_selection() {
                anyhow::bail!("{error}");
            }
        }
        Ok(())
    }

    /// `c` on the screen: split the queue into batches and start the first.
    fn continue_host_transfer(&mut self) -> Result<()> {
        let Some(pending) = self.pending_transfer.as_mut() else {
            return Ok(());
        };
        let Some(screen) = pending.screen.as_mut() else {
            return Ok(());
        };
        if screen.queue.is_empty() {
            screen.notice = Some("nothing staged: → pushes, ← pulls".to_string());
            return Ok(());
        }
        let queue = screen.queue.clone();
        let mut batches = batches_of(&queue);
        let first = batches.remove(0);
        pending.staged = queue;
        pending.screen = None;
        pending.batches = batches;
        if let Err(error) = self.start_batch(first) {
            // Back to the screen with the queue intact and the reason shown.
            let staged = self
                .pending_transfer
                .as_ref()
                .map(|p| p.staged.clone())
                .unwrap_or_default();
            self.open_host_transfer_screen(staged)?;
            if let Some(screen) = self
                .pending_transfer
                .as_mut()
                .and_then(|p| p.screen.as_mut())
            {
                screen.notice = Some(format!("{error:#}"));
            }
        }
        Ok(())
    }

    /// Keys of the dual-pane screen. `Esc` (outside a search) is handled by
    /// the caller and cancels the whole transfer.
    pub(crate) fn handle_key_host_transfer(&mut self, key: KeyEvent) -> Result<()> {
        // Ctrl/Alt chords belong to nobody here.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Ok(());
        }
        let Some(screen) = self
            .pending_transfer
            .as_mut()
            .and_then(|pending| pending.screen.as_mut())
        else {
            return Ok(());
        };
        let focus = screen.focus;
        if screen.pane(focus).searching {
            let pane = screen.pane_mut(focus);
            match key.code {
                KeyCode::Esc => {
                    pane.filter.clear();
                    pane.searching = false;
                }
                KeyCode::Enter => pane.searching = false,
                KeyCode::Backspace => {
                    pane.filter.pop();
                }
                KeyCode::Char(c) if !c.is_control() => pane.filter.push(c),
                _ => {}
            }
            pane.selected = 0;
            return Ok(());
        }
        screen.notice = None;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => screen.pane_mut(focus).move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => screen.pane_mut(focus).move_by(1),
            KeyCode::PageUp => screen.pane_mut(focus).move_by(-10),
            KeyCode::PageDown => screen.pane_mut(focus).move_by(10),
            KeyCode::Tab | KeyCode::BackTab => {
                screen.focus = match focus {
                    Side::Left => Side::Right,
                    Side::Right => Side::Left,
                }
            }
            KeyCode::Char('/') => screen.pane_mut(focus).searching = true,
            KeyCode::Right => match focus {
                Side::Left => {
                    let hosts = screen.pane(focus).selected_hosts();
                    screen.stage(hosts);
                }
                Side::Right => {
                    screen.notice = Some("→ pushes from the left pane (Tab); ← pulls".to_string())
                }
            },
            KeyCode::Left => match focus {
                Side::Right => {
                    let hosts = screen.pane(focus).selected_hosts();
                    screen.stage(hosts);
                }
                Side::Left => {
                    screen.notice = Some("← pulls from the right pane (Tab); → pushes".to_string())
                }
            },
            KeyCode::Enter | KeyCode::Char(' ') => {
                let hosts = screen.pane(focus).selected_hosts();
                screen.stage(hosts);
            }
            KeyCode::Char('a') => {
                let hosts = screen.pane(focus).shown_hosts();
                screen.stage(hosts);
            }
            KeyCode::Char('u') => {
                screen.queue.pop();
            }
            KeyCode::Char('x') => screen.queue.clear(),
            KeyCode::Char('c') => return self.continue_host_transfer(),
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{LauncherStore, NewHost, NewHostGroup};

    fn pane() -> Pane {
        let store = LauncherStore::open_in_memory().unwrap();
        let prod = store
            .create_group(&NewHostGroup {
                name: "prod".into(),
                ..Default::default()
            })
            .unwrap()
            .id;
        for (name, group_id) in [
            ("web2", Some(prod)),
            ("web1", Some(prod)),
            ("lab", None),
            ("db", Some(prod)),
        ] {
            store
                .create_host(&NewHost {
                    group_id,
                    ..NewHost::launcher(name, format!("{name}.example.com"))
                })
                .unwrap();
        }
        Pane::new("work".into(), &TransferSnapshot::capture(&store).unwrap())
    }

    fn names(pane: &Pane) -> Vec<String> {
        pane.rows()
            .iter()
            .map(|row| match row {
                PaneRow::Group { name, count } => format!("[{name} {count}]"),
                PaneRow::Host(i) => pane.hosts[*i].name.clone(),
            })
            .collect()
    }

    #[test]
    fn rows_group_hosts_alphabetically_with_ungrouped_last() {
        assert_eq!(
            names(&pane()),
            ["[prod 3]", "db", "web1", "web2", "[(no group) 1]", "lab"]
        );
    }

    #[test]
    fn filter_keeps_only_matching_hosts_and_their_groups() {
        let mut p = pane();
        p.filter = "WEB".into();
        assert_eq!(names(&p), ["[prod 2]", "web1", "web2"]);
        p.filter = "example.com".into(); // matches the address
        assert_eq!(p.shown(), 4);
        p.filter = "zzz".into();
        assert!(p.rows().is_empty() && p.selected_hosts().is_empty());
    }

    #[test]
    fn a_group_row_selects_every_shown_host_of_the_group() {
        let mut p = pane();
        p.selected = 0; // [prod 3]
        assert_eq!(p.selected_hosts(), ["db", "web1", "web2"]);
        p.filter = "web".into();
        p.selected = 0;
        assert_eq!(p.selected_hosts(), ["web1", "web2"], "only what is shown");
        p.selected = 1;
        assert_eq!(p.selected_hosts(), ["web1"]);
    }

    #[test]
    fn batches_split_by_direction_push_first_and_keep_order() {
        let queue = vec![
            Staged {
                pull: true,
                host: "p1".into(),
            },
            Staged {
                pull: false,
                host: "a".into(),
            },
            Staged {
                pull: false,
                host: "b".into(),
            },
            Staged {
                pull: true,
                host: "p2".into(),
            },
        ];
        let batches = batches_of(&queue);
        assert_eq!(batches.len(), 2);
        assert!(!batches[0].pull && batches[0].hosts == ["a", "b"]);
        assert!(batches[1].pull && batches[1].hosts == ["p1", "p2"]);
        assert!(batches_of(&[]).is_empty());
    }
}
