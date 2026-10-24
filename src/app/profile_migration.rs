//! Guided profile-to-profile migration: the `m` choice of the transfer dialog.
//!
//! Five checklist steps: hosts, groups of the chosen hosts, tunnels of the
//! chosen hosts, identities they (or the chosen groups) use, audit events of
//! the chosen hosts, then the usual plan review and explicit `y`. A step
//! whose list is empty is skipped. The wizard only collects a
//! [`TransferSelection`]; planning and applying stay in the engine
//! ([`crate::store::transfer`]).

use std::collections::{HashMap, HashSet};

use crate::store::transfer::{TransferSelection, TransferSnapshot};

/// Checklist rows shown at once; the list scrolls around the cursor.
const VISIBLE_ROWS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationStep {
    Hosts,
    Groups,
    Tunnels,
    Identities,
    Audit,
}

impl MigrationStep {
    const ORDER: [MigrationStep; 5] = [
        MigrationStep::Hosts,
        MigrationStep::Groups,
        MigrationStep::Tunnels,
        MigrationStep::Identities,
        MigrationStep::Audit,
    ];

    fn index(self) -> usize {
        Self::ORDER.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn title(self) -> &'static str {
        match self {
            MigrationStep::Hosts => "Hosts",
            MigrationStep::Groups => "Groups of the chosen hosts",
            MigrationStep::Tunnels => "Tunnels of the chosen hosts",
            MigrationStep::Identities => "Identities of the chosen hosts",
            MigrationStep::Audit => "Audit logs of the chosen hosts",
        }
    }
}

/// Where [`MigrationWizard::advance`] landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// Moved to the next non-empty step.
    Step,
    /// No step left: the selection is complete, review the plan.
    Review,
}

#[derive(Debug, Clone)]
pub struct CheckRow {
    /// Engine key: host/identity name, or a tunnel token.
    pub key: String,
    pub label: String,
    pub checked: bool,
}

pub struct MigrationWizard {
    pub step: MigrationStep,
    snapshot: TransferSnapshot,
    audit_counts: HashMap<String, usize>,
    /// Steps already left, so `back` revisits exactly the steps shown.
    history: Vec<MigrationStep>,
    rows: [Vec<CheckRow>; 5],
    /// `(step index, key)` of every row the user unchecked on a dependent
    /// step; outlives the rows themselves.
    unchecked: HashSet<(usize, String)>,
    cursor: usize,
}

impl MigrationWizard {
    pub fn new(snapshot: TransferSnapshot, audit_counts: HashMap<String, usize>) -> Self {
        let mut hosts: Vec<CheckRow> = snapshot
            .hosts
            .iter()
            .map(|host| CheckRow {
                key: host.name.clone(),
                label: if host.address.is_empty() || host.address == host.name {
                    host.name.clone()
                } else {
                    format!("{} · {}", host.name, host.address)
                },
                checked: false,
            })
            .collect();
        hosts.sort_by(|a, b| a.key.cmp(&b.key));
        Self {
            step: MigrationStep::Hosts,
            snapshot,
            audit_counts,
            history: Vec::new(),
            rows: [hosts, Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            unchecked: HashSet::new(),
            cursor: 0,
        }
    }

    /// Check these hosts on the first step (names the profile lacks are ignored).
    pub fn preselect(&mut self, names: &[String]) {
        for row in &mut self.rows[MigrationStep::Hosts.index()] {
            row.checked = names.contains(&row.key);
        }
    }

    fn rows(&self) -> &Vec<CheckRow> {
        &self.rows[self.step.index()]
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.rows().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn toggle(&mut self) {
        let cursor = self.cursor;
        let step = self.step.index();
        let Some(row) = self.rows[step].get_mut(cursor) else {
            return;
        };
        row.checked = !row.checked;
        let (key, now) = (row.key.clone(), row.checked);
        self.remember((step, key), now);
    }

    pub fn set_all(&mut self, checked: bool) {
        let step = self.step.index();
        for row in &mut self.rows[step] {
            row.checked = checked;
        }
        let keys: Vec<String> = self.rows[step].iter().map(|r| r.key.clone()).collect();
        for key in keys {
            self.remember((step, key), checked);
        }
    }

    /// Record a dependent-step choice (the host step never rebuilds).
    fn remember(&mut self, entry: (usize, String), checked: bool) {
        if entry.0 == 0 {
            return;
        }
        if checked {
            self.unchecked.remove(&entry);
        } else {
            self.unchecked.insert(entry);
        }
    }

    fn checked_keys(&self, step: MigrationStep) -> Vec<String> {
        self.rows[step.index()]
            .iter()
            .filter(|row| row.checked)
            .map(|row| row.key.clone())
            .collect()
    }

    /// Rows of `step` for the current host choice. Everything defaults to
    /// checked: a chosen host brings its tunnels, identity and audit trail
    /// unless the user unchecks them.
    fn build_rows(&self, step: MigrationStep) -> Vec<CheckRow> {
        let host_names: HashSet<String> = self
            .checked_keys(MigrationStep::Hosts)
            .into_iter()
            .collect();
        let hosts: Vec<_> = self
            .snapshot
            .hosts
            .iter()
            .filter(|host| host_names.contains(&host.name))
            .collect();
        match step {
            MigrationStep::Hosts => Vec::new(),
            MigrationStep::Groups => {
                // Real groups only: Favorites is per profile and never travels.
                let mut members: HashMap<i64, usize> = HashMap::new();
                for host in &hosts {
                    for group in host.groups.iter().filter(|g| !g.reserved) {
                        *members.entry(group.id).or_default() += 1;
                    }
                }
                let mut rows: Vec<CheckRow> = self
                    .snapshot
                    .groups
                    .iter()
                    .filter(|g| !g.reserved)
                    .filter_map(|group| {
                        let n = members.get(&group.id)?;
                        Some(CheckRow {
                            key: group.name.clone(),
                            label: format!(
                                "{} ({n} chosen host{})",
                                group.name,
                                if *n == 1 { "" } else { "s" }
                            ),
                            checked: true,
                        })
                    })
                    .collect();
                rows.sort_by(|a, b| a.key.cmp(&b.key));
                rows
            }
            MigrationStep::Tunnels => {
                let by_id: HashMap<i64, &str> =
                    hosts.iter().map(|h| (h.id, h.name.as_str())).collect();
                self.snapshot
                    .tunnels
                    .iter()
                    .filter_map(|t| {
                        let host = by_id.get(&t.host_id?)?;
                        // The id token picks exactly this tunnel; a label may be
                        // shared with tunnels of hosts that are not migrating.
                        let key = format!("tunnel#{}", t.id);
                        let name = match &t.label {
                            Some(label) if !label.trim().is_empty() => label.as_str(),
                            _ => key.as_str(),
                        };
                        Some(CheckRow {
                            label: format!(
                                "{name} · {} :{} → {}:{} · {host}",
                                t.tunnel_type.as_str(),
                                t.local_port,
                                t.remote_host,
                                t.remote_port
                            ),
                            key,
                            checked: true,
                        })
                    })
                    .collect()
            }
            MigrationStep::Identities => {
                let mut users: HashMap<i64, usize> = HashMap::new();
                for host in &hosts {
                    if let Some(id) = host.identity_id {
                        *users.entry(id).or_default() += 1;
                    }
                }
                // The default identity of a chosen group is wanted too: the
                // group lands with it, and its members inherit it.
                let chosen_groups: HashSet<String> = self
                    .checked_keys(MigrationStep::Groups)
                    .into_iter()
                    .collect();
                let group_defaults: HashSet<i64> = self
                    .snapshot
                    .groups
                    .iter()
                    .filter(|g| chosen_groups.contains(&g.name))
                    .filter_map(|g| g.default_identity_id)
                    .collect();
                let mut rows: Vec<CheckRow> = self
                    .snapshot
                    .identities
                    .iter()
                    .filter_map(|identity| {
                        let n = users.get(&identity.id).copied().unwrap_or(0);
                        let by_group = group_defaults.contains(&identity.id);
                        if n == 0 && !by_group {
                            return None;
                        }
                        let mut label = format!(
                            "{} ({n} host{}",
                            identity.name,
                            if n == 1 { "" } else { "s" }
                        );
                        if by_group {
                            label.push_str(", group default");
                        }
                        label.push(')');
                        Some(CheckRow {
                            key: identity.name.clone(),
                            label,
                            checked: true,
                        })
                    })
                    .collect();
                rows.sort_by(|a, b| a.key.cmp(&b.key));
                rows
            }
            MigrationStep::Audit => {
                let mut rows: Vec<CheckRow> = hosts
                    .iter()
                    .filter_map(|host| {
                        let n = *self.audit_counts.get(&host.name)?;
                        Some(CheckRow {
                            key: host.name.clone(),
                            label: format!(
                                "{} ({n} event{})",
                                host.name,
                                if n == 1 { "" } else { "s" }
                            ),
                            checked: true,
                        })
                    })
                    .collect();
                rows.sort_by(|a, b| a.key.cmp(&b.key));
                rows
            }
        }
    }

    /// Leave the current step. Needs at least one host on the first step;
    /// later steps may be emptied (that leaves the dependents behind).
    pub fn advance(&mut self) -> Result<Advance, &'static str> {
        if self.step == MigrationStep::Hosts && self.checked_keys(MigrationStep::Hosts).is_empty() {
            return Err("select at least one host");
        }
        let from = self.step.index();
        // Every later step is rebuilt for the current host choice, keeping
        // whatever the user unchecked, even if its row was absent meanwhile.
        for step in MigrationStep::ORDER[from + 1..].iter().copied() {
            let mut rows = self.build_rows(step);
            for row in &mut rows {
                row.checked = !self.unchecked.contains(&(step.index(), row.key.clone()));
            }
            self.rows[step.index()] = rows;
        }
        for next in MigrationStep::ORDER[from + 1..].iter().copied() {
            if self.rows[next.index()].is_empty() {
                continue;
            }
            self.history.push(self.step);
            self.step = next;
            self.cursor = 0;
            return Ok(Advance::Step);
        }
        Ok(Advance::Review)
    }

    /// Revisit the previous step; `false` on the first one.
    pub fn back(&mut self) -> bool {
        match self.history.pop() {
            Some(prev) => {
                self.step = prev;
                self.cursor = 0;
                true
            }
            None => false,
        }
    }

    /// What a migrated host will not bring along, for the plan review: a group
    /// or identity the user unchecked stays behind. A move deletes the source
    /// host, so the user should read this before `y`.
    pub fn losses(&self) -> Vec<String> {
        let chosen: HashSet<String> = self
            .checked_keys(MigrationStep::Hosts)
            .into_iter()
            .collect();
        let chosen_groups: HashSet<String> = self
            .checked_keys(MigrationStep::Groups)
            .into_iter()
            .collect();
        let kept_identities: HashSet<String> = self
            .checked_keys(MigrationStep::Identities)
            .into_iter()
            .collect();
        let identity_name: HashMap<i64, &str> = self
            .snapshot
            .identities
            .iter()
            .map(|i| (i.id, i.name.as_str()))
            .collect();
        let mut ungrouped = Vec::new();
        let mut unlinked = Vec::new();
        for host in self
            .snapshot
            .hosts
            .iter()
            .filter(|h| chosen.contains(&h.name))
        {
            if host
                .groups
                .iter()
                .any(|g| !g.reserved && !chosen_groups.contains(&g.name))
            {
                ungrouped.push(host.name.clone());
            }
            let identity = host.identity_id.and_then(|id| identity_name.get(&id));
            if identity.is_some_and(|name| !kept_identities.contains(*name)) {
                unlinked.push(host.name.clone());
            }
        }
        // A chosen group whose default identity was unchecked: its members
        // lose the login they inherited.
        let lost_defaults: Vec<String> = self
            .snapshot
            .groups
            .iter()
            .filter(|g| chosen_groups.contains(&g.name))
            .filter(|g| {
                g.default_identity_id
                    .and_then(|id| identity_name.get(&id))
                    .is_some_and(|name| !kept_identities.contains(*name))
            })
            .map(|g| g.name.clone())
            .collect();
        let mut notes = Vec::new();
        if !lost_defaults.is_empty() {
            notes.push(format!(
                "note: group {} land without their default identity (unchecked)",
                lost_defaults.join(", ")
            ));
        }
        for (names, what) in [
            (ungrouped, "land without a group (unchecked)"),
            (unlinked, "land without their identity (unchecked)"),
        ] {
            if names.is_empty() {
                continue;
            }
            let more = names.len().saturating_sub(3);
            let shown = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            notes.push(if more > 0 {
                format!("note: {shown} and {more} more {what}")
            } else {
                format!("note: {shown} {what}")
            });
        }
        notes
    }

    /// What the checklists amount to, for the engine. `no_auto_carry`: only
    /// the checked rows travel, so an unchecked identity or tunnel stays home.
    pub fn selection(&self) -> TransferSelection {
        TransferSelection {
            host_names: self.checked_keys(MigrationStep::Hosts),
            group_names: self.checked_keys(MigrationStep::Groups),
            identity_names: self.checked_keys(MigrationStep::Identities),
            tunnel_names: self.checked_keys(MigrationStep::Tunnels),
            audit_hosts: self.checked_keys(MigrationStep::Audit),
            no_auto_carry: true,
        }
    }

    /// Dialog body for the current step.
    pub fn lines(&self, other: &str, pull: bool) -> Vec<String> {
        let rows = self.rows();
        let checked = rows.iter().filter(|r| r.checked).count();
        let mut lines = vec![format!(
            "{} '{other}' · step {}/{}: {} ({checked}/{} selected)",
            if pull { "Pull from" } else { "Migrate to" },
            self.step.index() + 1,
            MigrationStep::ORDER.len(),
            self.step.title(),
            rows.len()
        )];
        let first = self
            .cursor
            .saturating_sub(VISIBLE_ROWS - 1)
            .min(rows.len().saturating_sub(VISIBLE_ROWS));
        if first > 0 {
            lines.push(format!("  … {first} above"));
        }
        for (i, row) in rows.iter().enumerate().skip(first).take(VISIBLE_ROWS) {
            lines.push(format!(
                "{}[{}] {}",
                if i == self.cursor { "▸ " } else { "  " },
                if row.checked { 'x' } else { ' ' },
                row.label
            ));
        }
        let below = rows.len().saturating_sub(first + VISIBLE_ROWS);
        if below > 0 {
            lines.push(format!("  … {below} more"));
        }
        // On the first step `b` returns to the dual-pane screen.
        let back = if self.history.is_empty() {
            "b back to hosts"
        } else {
            "b back"
        };
        lines.push("↑↓ move · Space toggle · a all · n none".to_string());
        lines.push(format!("Enter next · {back} · Esc cancel"));
        lines
    }
}

impl crate::app::App {
    /// Turn the wizard's checked rows into the plan: sets the migration
    /// scope and builds the plan review. Returns the error to show when the
    /// plan cannot be built (the scope is reset so the wizard stays open).
    pub(crate) fn finish_migration_selection(&mut self) -> Option<String> {
        let selection = self
            .pending_transfer
            .as_ref()
            .and_then(|pending| pending.wizard.as_ref())
            .map(MigrationWizard::selection)
            .unwrap_or_default();
        if let Some(pending) = self.pending_transfer.as_mut() {
            pending.scope = Some(super::profile_transfer::TransferScope::Migration(selection));
            pending.plan = None;
            pending.dialog = None;
        }
        if let Err(error) = self.rebuild_transfer_plan() {
            if let Some(pending) = self.pending_transfer.as_mut() {
                pending.scope = None;
            }
            return Some(format!("{error:#}"));
        }
        None
    }

    /// Keys of a checklist step. `Esc` is handled by the caller.
    pub(crate) fn handle_key_migration_wizard(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> anyhow::Result<()> {
        use crossterm::event::KeyCode;
        // Ctrl/Alt chords belong to nobody here: Ctrl+N must not read as `n`.
        if key.modifiers.intersects(
            crossterm::event::KeyModifiers::CONTROL | crossterm::event::KeyModifiers::ALT,
        ) {
            return Ok(());
        }
        let Some(wizard) = self
            .pending_transfer
            .as_mut()
            .and_then(|pending| pending.wizard.as_mut())
        else {
            return Ok(());
        };
        let mut notice: Option<String> = None;
        let mut review = false;
        let mut leave = false;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => wizard.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => wizard.move_cursor(1),
            KeyCode::Char(' ') => wizard.toggle(),
            KeyCode::Char('a') => wizard.set_all(true),
            KeyCode::Char('n') => wizard.set_all(false),
            // Only `b` leaves the wizard from its first step; a stray arrow or
            // Backspace must not drop a long host selection.
            KeyCode::Char('b') => leave = !wizard.back(),
            KeyCode::Left | KeyCode::Backspace => {
                wizard.back();
            }
            KeyCode::Enter => match wizard.advance() {
                Ok(Advance::Step) => {}
                Ok(Advance::Review) => review = true,
                Err(message) => notice = Some(message.into()),
            },
            _ => {}
        }
        if leave {
            // First step: back to the dual-pane screen, queue intact.
            let staged = self
                .pending_transfer
                .as_ref()
                .map(|p| p.staged.clone())
                .unwrap_or_default();
            if let Err(error) = self.open_host_transfer_screen(staged) {
                notice = Some(format!("{error:#}"));
            }
        }
        if review {
            notice = self.finish_migration_selection();
        }
        if let Some(pending) = self.pending_transfer.as_mut() {
            pending.notice = notice;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{LauncherStore, NewHost, NewIdentity, NewTunnel, TunnelType};

    /// `a`: identity + tunnel + audit events; `b`: nothing attached.
    fn wizard() -> MigrationWizard {
        let store = LauncherStore::open_in_memory().unwrap();
        let identity = store
            .create_identity(&NewIdentity {
                name: "deploy".into(),
                username: None,
                private_key: None,
                certificate: None,
                sort_order: 0,
                has_password: false,
            })
            .unwrap();
        for (name, identity_id) in [("a", Some(identity.id)), ("b", None)] {
            store
                .create_host(&NewHost {
                    identity_id,
                    ..NewHost::launcher(name, "10.0.0.1")
                })
                .unwrap();
        }
        let a = store.get_host_by_name("a").unwrap().unwrap().id;
        store
            .create_tunnel(&NewTunnel {
                host_id: Some(a),
                tunnel_type: TunnelType::Local,
                local_port: 8080,
                remote_host: "db".into(),
                remote_port: 5432,
                label: Some("pg".into()),
                auto_connect: false,
            })
            .unwrap();
        store
            .log_auth_event("a", None, "tui", "ok", "", None)
            .unwrap();
        MigrationWizard::new(
            TransferSnapshot::capture(&store).unwrap(),
            store.auth_event_counts().unwrap(),
        )
    }

    fn check(wizard: &mut MigrationWizard, key: &str) {
        let index = wizard.rows().iter().position(|r| r.key == key).unwrap();
        wizard.cursor = index;
        wizard.toggle();
    }

    #[test]
    fn the_first_step_names_where_b_goes() {
        let mut w = wizard();
        let hint = |w: &MigrationWizard| w.lines("work", false).last().cloned().unwrap();
        assert!(hint(&w).contains("b back to hosts"), "{}", hint(&w));
        check(&mut w, "a");
        w.advance().unwrap();
        assert!(!hint(&w).contains("to hosts"), "{}", hint(&w));
    }

    #[test]
    fn steps_follow_the_chosen_hosts_and_skip_empty_ones() {
        let mut w = wizard();
        assert_eq!(w.advance(), Err("select at least one host"));
        check(&mut w, "a");
        assert_eq!(w.advance(), Ok(Advance::Step));
        assert_eq!(w.step, MigrationStep::Tunnels);
        assert_eq!(w.advance(), Ok(Advance::Step));
        assert_eq!(w.step, MigrationStep::Identities);
        assert_eq!(w.advance(), Ok(Advance::Step));
        assert_eq!(w.step, MigrationStep::Audit);
        assert_eq!(w.advance(), Ok(Advance::Review));
        let s = w.selection();
        assert_eq!(s.host_names, ["a"]);
        assert!(matches!(s.tunnel_names.as_slice(), [t] if t.starts_with("tunnel#")));
        assert_eq!(s.identity_names, ["deploy"]);
        assert_eq!(s.audit_hosts, ["a"]);

        // `b` has no tunnel, identity or events: straight to review.
        let mut w = wizard();
        check(&mut w, "b");
        assert_eq!(w.advance(), Ok(Advance::Review));
        assert_eq!(w.selection().host_names, ["b"]);
        assert!(w.selection().tunnel_names.is_empty());
    }

    #[test]
    fn unchecked_rows_stay_out_and_stale_rows_do_not_survive_a_host_change() {
        let mut w = wizard();
        check(&mut w, "a");
        w.advance().unwrap(); // tunnels
        w.toggle(); // uncheck `pg`
        assert!(w.back());
        assert_eq!(w.step, MigrationStep::Hosts);
        assert!(!w.back(), "hosts is the first step");
        // Swap the host choice: `a`'s tunnel/identity/audit rows must vanish.
        check(&mut w, "a");
        check(&mut w, "b");
        assert_eq!(w.advance(), Ok(Advance::Review));
        let s = w.selection();
        assert_eq!(s.host_names, ["b"]);
        assert!(s.tunnel_names.is_empty() && s.identity_names.is_empty());
        assert!(s.audit_hosts.is_empty());

        // Uncheck the tunnel for real: it is left out of the selection.
        let mut w = wizard();
        check(&mut w, "a");
        w.advance().unwrap();
        w.toggle();
        assert_eq!(w.advance(), Ok(Advance::Step));
        assert!(w.selection().tunnel_names.is_empty());
        assert_eq!(w.selection().identity_names, ["deploy"]);
    }

    #[test]
    fn going_back_and_forward_keeps_what_was_unchecked() {
        let mut w = wizard();
        check(&mut w, "a");
        w.advance().unwrap(); // tunnels
        w.advance().unwrap(); // identities
        w.toggle(); // uncheck `deploy`
        assert!(w.back()); // tunnels
        assert_eq!(w.advance(), Ok(Advance::Step)); // identities again
        assert_eq!(w.step, MigrationStep::Identities);
        assert!(w.selection().identity_names.is_empty(), "uncheck survived");
    }

    #[test]
    fn unchecks_survive_going_back_two_steps_and_a_host_dropped_in_between() {
        let mut w = wizard();
        check(&mut w, "a");
        w.advance().unwrap(); // tunnels
        w.advance().unwrap(); // identities
        w.toggle(); // uncheck `deploy`
        w.advance().unwrap(); // audit
        w.toggle(); // uncheck `a (1 event)`
        assert!(w.back() && w.back() && w.back()); // identities, tunnels, hosts
        assert_eq!(w.step, MigrationStep::Hosts);
        // Drop `a`, go forward (its rows vanish), take `a` back, go forward.
        check(&mut w, "a");
        check(&mut w, "b");
        assert_eq!(w.advance(), Ok(Advance::Review));
        w.back();
        check(&mut w, "a");
        check(&mut w, "b");
        while w.advance() == Ok(Advance::Step) {}
        let s = w.selection();
        assert!(s.identity_names.is_empty(), "deploy stayed unchecked");
        assert!(s.audit_hosts.is_empty(), "audit stayed unchecked");
        assert_eq!(s.tunnel_names.len(), 1, "an untouched row stays checked");
    }

    /// `c` sits in group `prod` (default identity `deploy`) and has no identity
    /// of its own; `d` is ungrouped.
    fn grouped_wizard() -> MigrationWizard {
        let store = LauncherStore::open_in_memory().unwrap();
        let identity = store
            .create_identity(&NewIdentity {
                name: "deploy".into(),
                username: None,
                private_key: None,
                certificate: None,
                sort_order: 0,
                has_password: false,
            })
            .unwrap();
        let group = store
            .create_group(&crate::store::NewHostGroup {
                name: "prod".into(),
                default_identity_id: Some(identity.id),
                ..Default::default()
            })
            .unwrap();
        for (name, group_id) in [("c", Some(group.id)), ("d", None)] {
            store
                .create_host(&NewHost {
                    group_id,
                    ..NewHost::launcher(name, "10.0.0.1")
                })
                .unwrap();
        }
        MigrationWizard::new(
            TransferSnapshot::capture(&store).unwrap(),
            store.auth_event_counts().unwrap(),
        )
    }

    #[test]
    fn groups_step_lists_the_groups_of_the_chosen_hosts_and_feeds_identities() {
        let mut w = grouped_wizard();
        check(&mut w, "d");
        assert_eq!(w.advance(), Ok(Advance::Review), "`d` has no group");
        assert!(w.selection().group_names.is_empty());

        let mut w = grouped_wizard();
        check(&mut w, "c");
        assert_eq!(w.advance(), Ok(Advance::Step));
        assert_eq!(w.step, MigrationStep::Groups);
        assert_eq!(w.rows()[0].label, "prod (1 chosen host)");
        // No identity on the host itself, yet the group default is offered.
        assert_eq!(w.advance(), Ok(Advance::Step));
        assert_eq!(w.step, MigrationStep::Identities);
        assert_eq!(w.rows()[0].label, "deploy (0 hosts, group default)");
        assert_eq!(w.advance(), Ok(Advance::Review));
        let s = w.selection();
        assert_eq!(s.group_names, ["prod"]);
        assert_eq!(s.identity_names, ["deploy"]);
        assert!(w.losses().is_empty());
    }

    #[test]
    fn an_unchecked_group_drops_its_default_identity_and_is_noted() {
        let mut w = grouped_wizard();
        check(&mut w, "c");
        w.advance().unwrap(); // groups
        w.toggle(); // uncheck `prod`
        assert_eq!(
            w.advance(),
            Ok(Advance::Review),
            "no identity left to offer"
        );
        let s = w.selection();
        assert!(s.group_names.is_empty() && s.identity_names.is_empty());
        assert_eq!(
            w.losses(),
            ["note: c land without a group (unchecked)".to_string()]
        );
    }
}
