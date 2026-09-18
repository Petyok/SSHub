//! Cross-profile transfers from the profile manager.
//!
//! The `T` action in [`crate::app::App::handle_key_profile_picker`] drives a
//! staged flow (destination → scope → plan → explicit confirm) that delegates
//! all planning and application to the engine module
//! [`crate::store::transfer`]. This file owns rendering ([`plan_text`],
//! [`TransferDialog`]) and the flow state ([`PendingTransfer`]); it never
//! duplicates engine logic.

use std::collections::HashSet;

use anyhow::{Context, Result};
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};

use crate::store::transfer::{
    apply_transfer_plan, build_transfer_plan, DestIndex, GroupTransferMode, TransferMode,
    TransferPlan, TransferSelection, TransferSnapshot,
};
use crate::store::LauncherStore;

/// What to transfer: one entity by name. Groups carry the group-only vs
/// with-hosts choice (mirroring delete promotion: group-only keeps member
/// hosts in the source profile).
#[derive(Debug, Clone)]
pub enum TransferScope {
    Host(String),
    Group { name: String, with_hosts: bool },
    Identity(String),
    Tunnel(String),
}

impl TransferScope {
    pub fn kind(&self) -> &'static str {
        match self {
            TransferScope::Host(_) => "host",
            TransferScope::Group { .. } => "group",
            TransferScope::Identity(_) => "identity",
            TransferScope::Tunnel(_) => "tunnel",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            TransferScope::Host(name)
            | TransferScope::Group { name, .. }
            | TransferScope::Identity(name)
            | TransferScope::Tunnel(name) => name,
        }
    }

    fn selection(&self) -> TransferSelection {
        let mut selection = TransferSelection::default();
        match self {
            TransferScope::Host(name) => selection.host_names.push(name.clone()),
            TransferScope::Group { name, .. } => selection.group_names.push(name.clone()),
            TransferScope::Identity(name) => selection.identity_names.push(name.clone()),
            TransferScope::Tunnel(name) => selection.tunnel_names.push(name.clone()),
        }
        selection
    }

    fn group_mode(&self) -> GroupTransferMode {
        match self {
            TransferScope::Group {
                with_hosts: true, ..
            } => GroupTransferMode::WithHosts,
            _ => GroupTransferMode::GroupOnly,
        }
    }
}

/// Render a transfer plan, reusing the ConfirmDelete dialog style: one line
/// per item (renames and per-item blocked notes inline), closed with an
/// explicit `y` / `Esc` hint. The plan never applies without `y`.
pub fn plan_text(plan: &TransferPlan) -> String {
    let mut out = format!("Transfer to profile '{}':", plan.dest_profile);
    if plan.items.is_empty() {
        out.push_str("\n  (nothing selected)");
    }
    for item in &plan.items {
        let mut line = format!("  [{}] '{}'", item.kind, item.src_name);
        if item.dest_name != item.src_name {
            line.push_str(&format!(" -> '{}'", item.dest_name));
        }
        if item.renamed {
            line.push_str(" (renamed: name clash in destination)");
        }
        if let Some(reason) = &item.blocked {
            line.push_str(&format!(" — blocked: {reason}"));
        }
        out.push('\n');
        out.push_str(&line);
    }
    out.push_str(&format!(
        "\n{} to transfer, {} blocked — y: transfer    Esc: cancel",
        plan.runnable().len(),
        plan.blocked().len()
    ));
    out
}

/// Group transfers prompt group-only vs with-hosts. Group-only keeps member
/// hosts in the source profile (existing delete-promotion behavior, where the
/// members' primary group is recomputed); with-hosts moves/copies the member
/// hosts too.
pub fn group_scope_prompt(group_name: &str) -> String {
    format!("Group '{group_name}': g group-only (members stay) · w with-hosts (members move too)")
}

/// Transfers always require explicit confirmation, even with nothing blocked.
pub fn needs_explicit_confirm(_plan: &TransferPlan) -> bool {
    true
}

/// Plan dialog state: the rendered plan text awaiting an explicit `y`.
pub struct TransferDialog {
    pub plan_text: String,
}

impl TransferDialog {
    pub fn new(plan: &TransferPlan) -> Self {
        Self {
            plan_text: plan_text(plan),
        }
    }
}

/// Staged TUI transfer flow state, held on [`crate::app::App::pending_transfer`].
/// Rendered by the dedicated transfer popup (`render_profile_transfer_popup`),
/// never as inline picker text: every line below is ellipsized to the dialog
/// rect at render time.
pub struct PendingTransfer {
    /// Validated destination profile name. `None` while it is being picked.
    pub dest_name: Option<String>,
    /// Fuzzy query while choosing the destination.
    pub dest_buffer: String,
    /// Candidate destination names (every profile except the active one),
    /// snapshotted when the flow is armed.
    pub dest_candidates: Vec<String>,
    /// Indices into `dest_candidates` matching the query, best match first.
    pub dest_filtered: Vec<usize>,
    /// Cursor into `dest_filtered`.
    pub dest_selected: usize,
    /// Destination `launcher.db`, resolved once the name validates.
    pub dest_db: Option<std::path::PathBuf>,
    pub scope: Option<TransferScope>,
    pub is_move: bool,
    pub plan: Option<TransferPlan>,
    pub dialog: Option<TransferDialog>,
    /// Transient error from the last rejected key, shown as the dialog's error
    /// line until the next key clears it. Kept off the picker message line so
    /// long transfer text can never overflow the picker's bounds.
    pub notice: Option<String>,
}

/// Fuzzy-rank profile names against `query`, returning indices into `names`
/// best-match first. An empty query keeps registry order.
///
/// The same nucleo wiring the host search ([`crate::search::HostSearch`]) and
/// the snippet picker use — [`Pattern::parse`] with smart case/normalization
/// over a single name field — not a hand-rolled matcher.
pub(crate) fn rank_profile_names(names: &[String], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..names.len()).collect();
    }
    // Reuse one matcher across keystrokes (like `HostSearch`) rather than
    // rebuilding it on every query. The picker runs on the single-threaded
    // event loop; a thread-local keeps tests (which call this in parallel)
    // isolated — the same arrangement `rank_snippets` uses.
    thread_local! {
        static MATCHER: std::cell::RefCell<Matcher> =
            std::cell::RefCell::new(Matcher::new(Config::DEFAULT));
    }
    MATCHER.with(|cell| {
        let mut matcher = cell.borrow_mut();
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize)> = Vec::new();
        for (idx, name) in names.iter().enumerate() {
            buf.clear();
            if let Some(score) = pattern.score(Utf32Str::new(name, &mut buf), &mut matcher) {
                scored.push((score, idx));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, idx)| idx).collect()
    })
}

/// Rows of the destination picker shown in the dialog at once; the renderer
/// caps the popup to the frame anyway, but a short list keeps the dialog
/// compact on tall terminals too.
const DEST_PICKER_ROWS: usize = 6;

impl PendingTransfer {
    fn new() -> Self {
        Self {
            dest_name: None,
            dest_buffer: String::new(),
            dest_candidates: Vec::new(),
            dest_filtered: Vec::new(),
            dest_selected: 0,
            dest_db: None,
            scope: None,
            is_move: false,
            plan: None,
            dialog: None,
            notice: None,
        }
    }

    /// Re-run the fuzzy filter over the current query, clamping the cursor.
    /// Mirrors the snippet picker's rebuild (selection survives keystrokes
    /// when it still points at a match).
    pub fn refresh_dest_filter(&mut self) {
        self.dest_filtered = rank_profile_names(&self.dest_candidates, &self.dest_buffer);
        self.dest_selected = if self.dest_filtered.is_empty() {
            0
        } else {
            self.dest_selected.min(self.dest_filtered.len() - 1)
        };
    }

    /// Move the destination cursor, clamped to the current match set.
    /// Mirrors the snippet picker's move (empty set pins the cursor at 0).
    pub fn move_dest_selection(&mut self, delta: isize) {
        if self.dest_filtered.is_empty() {
            self.dest_selected = 0;
            return;
        }
        let max = self.dest_filtered.len() as isize - 1;
        self.dest_selected = (self.dest_selected as isize + delta).clamp(0, max) as usize;
    }

    /// The currently highlighted destination name, if any match remains.
    pub fn dest_highlight(&self) -> Option<&str> {
        self.dest_filtered
            .get(self.dest_selected)
            .and_then(|&idx| self.dest_candidates.get(idx))
            .map(String::as_str)
    }

    pub fn dialog_title(&self) -> &'static str {
        "Transfer profile"
    }

    /// One body line per dialog row, mirroring the staged keymap in
    /// [`crate::app::App::handle_key_profile_transfer`]: destination pick
    /// (type to filter, ↑↓ move, Enter choose) → scope pick (`h`/`g`/`i`/`t`)
    /// → group-only/with-hosts (`g`/`w`, group scope only) → plan review →
    /// explicit `y` confirm. Raw text: the popup renderer ellipsizes every
    /// line to its rect.
    pub fn dialog_lines(&self) -> Vec<String> {
        let mut lines = match (&self.dest_name, &self.scope, &self.plan) {
            (None, _, _) => {
                let mut lines = vec![
                    "Destination profile:".to_string(),
                    format!("> {}█", self.dest_buffer),
                ];
                if self.dest_filtered.is_empty() {
                    lines.push("(no matching profiles)".to_string());
                } else {
                    for (row, &idx) in self.dest_filtered.iter().take(DEST_PICKER_ROWS).enumerate()
                    {
                        let marker = if row == self.dest_selected {
                            "▸ "
                        } else {
                            "  "
                        };
                        lines.push(format!("{marker}{}", self.dest_candidates[idx]));
                    }
                    if self.dest_filtered.len() > DEST_PICKER_ROWS {
                        lines.push(format!(
                            "… {} more",
                            self.dest_filtered.len() - DEST_PICKER_ROWS
                        ));
                    }
                }
                lines.push("type to filter · ↑↓ select · Enter choose · Esc cancel".to_string());
                lines
            }
            (Some(dest), None, _) => vec![
                format!("Transfer what to '{dest}'?"),
                "h host · g group · i identity · t tunnel".to_string(),
                "Esc cancel".to_string(),
            ],
            (Some(_), Some(TransferScope::Group { name, .. }), None) => vec![
                group_scope_prompt(name),
                "g group-only · w with-hosts · Esc cancel".to_string(),
            ],
            (Some(_), Some(_), Some(plan)) => {
                let mut lines: Vec<String> = plan_text(plan).lines().map(str::to_string).collect();
                lines.push(format!(
                    "m: toggle move/copy (now {})",
                    if self.is_move { "move" } else { "copy" }
                ));
                lines
            }
            (Some(_), Some(_), None) => vec!["building plan…".to_string()],
        };
        if let Some(notice) = &self.notice {
            lines.push(format!("! {notice}"));
        }
        lines
    }
}

impl crate::app::App {
    /// Hosts that must not move: live (non-terminal) sessions plus hosts with
    /// a running/proving/reconnecting tunnel. Read from live UI state as
    /// names; the engine marks the matching plan items blocked while the rest
    /// still transfer.
    pub(crate) fn transfer_blocked_hosts(&self) -> HashSet<String> {
        let mut blocked = HashSet::new();
        for session in &self.sessions {
            if !session.phase.is_terminal() {
                blocked.insert(session.host_name.clone());
            }
        }
        for tunnel in &self.tunnels {
            if self.tunnel_manager.is_running(tunnel.id)
                || self.tunnel_manager.has_child(tunnel.id)
                || self.tunnel_manager.is_reconnecting(tunnel.id)
            {
                if let Some(host_id) = tunnel.host_id {
                    if let Ok(Some(host)) = self.store.get_host(host_id) {
                        blocked.insert(host.name);
                    }
                }
            }
        }
        blocked
    }

    pub(crate) fn arm_profile_transfer(&mut self) {
        let mut pending = PendingTransfer::new();
        match self.transfer_dest_candidates() {
            Ok(names) => {
                pending.dest_candidates = names;
                pending.refresh_dest_filter();
            }
            Err(error) => {
                pending.notice = Some(format!("{error:#}"));
            }
        }
        self.pending_transfer = Some(pending);
    }

    /// Every profile except the active one, in registry order: the active
    /// profile can never be a transfer destination (`choose_transfer_dest`
    /// refuses it), so the picker does not offer it.
    fn transfer_dest_candidates(&self) -> Result<Vec<String>> {
        let roots = self
            .profile_roots()
            .context("profile management is unavailable in compatibility mode")?;
        let state = crate::profile::ProfileState::load(&roots.data_root)?
            .context("profile registry is missing")?;
        let active = self.active_profile_name().to_string();
        Ok(state
            .profiles
            .iter()
            .map(|record| record.name.clone())
            .filter(|name| *name != active)
            .collect())
    }

    pub(crate) fn cancel_pending_transfer(&mut self) {
        self.pending_transfer = None;
    }

    /// Dashboard selection names used as transfer scopes.
    pub(crate) fn transfer_host_name(&self) -> Option<String> {
        self.hosts
            .get(self.selected)
            .map(|host| host.display_name().to_string())
    }

    pub(crate) fn transfer_group_name(&self) -> Option<String> {
        self.groups
            .get(self.group_manage_selected)
            .map(|group| group.name.clone())
    }

    pub(crate) fn transfer_identity_name(&self) -> Option<String> {
        self.identities
            .get(self.identity_selected)
            .map(|identity| identity.name.clone())
    }

    pub(crate) fn transfer_tunnel_name(&self) -> Option<String> {
        self.tunnels.get(self.tunnel_selected).map(|tunnel| {
            tunnel
                .label
                .clone()
                .unwrap_or_else(|| format!(":{}", tunnel.local_port))
        })
    }

    /// Validate the typed destination profile name and resolve its database.
    /// Refuses the active profile and profiles without a database yet.
    pub(crate) fn choose_transfer_dest(&mut self, raw: &str) -> Result<()> {
        let name = raw.trim();
        anyhow::ensure!(!name.is_empty(), "type a destination profile name");
        let roots = self
            .profile_roots()
            .context("profile management is unavailable in compatibility mode")?;
        let state = crate::profile::ProfileState::load(&roots.data_root)?
            .context("profile registry is missing")?;
        let record = state
            .by_name(name)
            .with_context(|| format!("no profile named '{name}'"))?;
        let active = self.active_profile_name().to_string();
        anyhow::ensure!(
            record.name != active,
            "cannot transfer into the active profile"
        );
        let db = crate::profile::ProfilePaths::profile_dir(&roots.data_root, &record.name)
            .join("launcher.db");
        anyhow::ensure!(
            db.exists(),
            "destination profile '{}' has no database yet",
            record.name
        );
        if let Some(pending) = self.pending_transfer.as_mut() {
            pending.dest_name = Some(record.name.clone());
            pending.dest_db = Some(db);
        }
        Ok(())
    }

    /// Record the scope; group scopes wait for the group-only vs with-hosts
    /// choice, everything else plans immediately.
    pub(crate) fn choose_transfer_scope(&mut self, scope: TransferScope) -> Result<()> {
        let needs_group_choice = matches!(scope, TransferScope::Group { .. });
        if let Some(pending) = self.pending_transfer.as_mut() {
            pending.scope = Some(scope);
            pending.plan = None;
            pending.dialog = None;
        }
        if !needs_group_choice {
            self.rebuild_transfer_plan()?;
        }
        Ok(())
    }

    pub(crate) fn set_transfer_with_hosts(&mut self, with_hosts: bool) -> Result<()> {
        if let Some(pending) = self.pending_transfer.as_mut() {
            if let Some(TransferScope::Group {
                with_hosts: flag, ..
            }) = pending.scope.as_mut()
            {
                *flag = with_hosts;
            }
        }
        self.rebuild_transfer_plan()
    }

    pub(crate) fn toggle_transfer_move(&mut self) -> Result<()> {
        if let Some(pending) = self.pending_transfer.as_mut() {
            pending.is_move = !pending.is_move;
        }
        self.rebuild_transfer_plan()
    }

    /// Snapshot the source (DTO copies, locks dropped between calls), the
    /// destination index, then delegate planning to the engine. Never holds
    /// two store locks at once.
    pub(crate) fn rebuild_transfer_plan(&mut self) -> Result<()> {
        let (dest_name, dest_db, scope, is_move) = match self.pending_transfer.as_ref() {
            Some(pending) => (
                pending
                    .dest_name
                    .clone()
                    .context("choose a destination profile first")?,
                pending
                    .dest_db
                    .clone()
                    .context("choose a destination profile first")?,
                pending
                    .scope
                    .clone()
                    .context("choose a transfer scope first")?,
                pending.is_move,
            ),
            None => anyhow::bail!("no transfer in progress"),
        };
        let snapshot = TransferSnapshot::capture(&self.store)?;
        let dest_store =
            LauncherStore::open(&dest_db).context("cannot open destination profile database")?;
        let index = DestIndex::capture(&dest_store, dest_name)?;
        let blocked = self.transfer_blocked_hosts();
        let mode = if is_move {
            TransferMode::Move
        } else {
            TransferMode::Copy
        };
        let plan = build_transfer_plan(
            &snapshot,
            &index,
            &scope.selection(),
            mode,
            scope.group_mode(),
            &blocked,
        );
        let dialog = TransferDialog::new(&plan);
        if let Some(pending) = self.pending_transfer.as_mut() {
            pending.plan = Some(plan);
            pending.dialog = Some(dialog);
        }
        Ok(())
    }

    /// Apply the confirmed plan through fresh store handles (owned `&mut`,
    /// never the live `Arc` handles, so no store lock is held across the
    /// copy), then reload the dashboard from the live handles.
    pub(crate) fn apply_pending_transfer(&mut self) -> Result<()> {
        let (dest_name, dest_db, is_move, plan) = match self.pending_transfer.as_mut() {
            Some(pending) => (
                pending.dest_name.clone().unwrap_or_default(),
                pending.dest_db.clone().unwrap_or_default(),
                pending.is_move,
                pending.plan.take().context("nothing planned yet")?,
            ),
            None => anyhow::bail!("no transfer in progress"),
        };
        let src_db = self
            .profile
            .as_ref()
            .context("no active profile")?
            .launcher_db();
        let mut src_store =
            LauncherStore::open(&src_db).context("cannot open source profile database")?;
        let mut dest_store =
            LauncherStore::open(&dest_db).context("cannot open destination profile database")?;
        let mode = if is_move {
            TransferMode::Move
        } else {
            TransferMode::Copy
        };
        let result = apply_transfer_plan(&mut src_store, &mut dest_store, &plan, mode);
        self.reload_hosts()?;
        self.reload_identities()?;
        self.reload_tunnels()?;
        let mut notice = format!(
            "Transferred {} item(s) to '{dest_name}'",
            result.transferred
        );
        if !result.renamed.is_empty() {
            notice.push_str(&format!(" (renamed: {})", result.renamed.join(", ")));
        }
        if !result.blocked.is_empty() {
            notice.push_str(&format!(" (blocked: {})", result.blocked.join(", ")));
        }
        self.host_notice = Some(notice);
        self.host_notice_at = Some(std::time::Instant::now());
        self.pending_transfer = None;
        Ok(())
    }
}
