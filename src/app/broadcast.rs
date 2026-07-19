//! Broadcast-mode app wiring (issue #3): the pre-run wizard (pick target →
//! command → preview) and the live background run's tick/cancel plumbing.
//!
//! The pure fan-out engine (pool, reducer, view helpers) lives in
//! [`crate::broadcast`]; this module is the `App`-facing glue: it builds the
//! wizard state, resolves a target to managed-host candidates, spawns the run,
//! drains its events each poll tick, folds them into the row table, writes one
//! audit row per host at completion, and drives the settle/pause/dismiss
//! lifecycle of the docked panel.

use super::*;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

impl App {
    /// True while the broadcast panel's entry slide is still playing. The main
    /// loop polls at a higher frame rate while this holds so the ~350ms slide
    /// looks smooth instead of stepping at the idle 20fps poll cadence.
    pub(crate) fn animating(&self) -> bool {
        let now = Instant::now();
        let panel = self
            .broadcast
            .as_ref()
            .and_then(|b| b.anim)
            .is_some_and(|a| !a.is_done(now));
        // A toast is animating during its slide-in (first TOAST_ANIM) or its
        // slide-out (the TOAST_ANIM after TOAST_TTL).
        let toasts = self.broadcast_toasts.iter().any(|t| {
            let e = now.saturating_duration_since(t.born);
            e < crate::broadcast::TOAST_ANIM
                || (e >= crate::broadcast::TOAST_TTL
                    && e < crate::broadcast::TOAST_TTL + crate::broadcast::TOAST_ANIM)
        });
        panel || toasts
    }

    /// Open the broadcast wizard from the hosts tab. Refuses while a run is live
    /// (one at a time). Builds the target menu from every group plus the sorted,
    /// deduped set of host tags; if there's nothing to target, surfaces a notice
    /// and stays put.
    pub(crate) fn open_broadcast(&mut self) {
        // Only an actively-running fleet blocks a new one. A finished panel
        // (settling / paused / leaving) is just dropped so you can fire again.
        if self
            .broadcast
            .as_ref()
            .is_some_and(|b| !crate::broadcast::all_terminal(&b.results))
        {
            self.host_notice = Some("A broadcast run is already in progress.".into());
            return;
        }
        // A finished panel still on screen: send it off with the exit slide
        // rather than yanking it, then open the wizard over it. tick_broadcast
        // removes it once the slide ends.
        if self.broadcast.is_some() {
            self.slide_broadcast_out();
        }

        // `self.groups` is the real user-group list (the reserved Favorites
        // group is deliberately kept out of it), matching the group-manage view.
        let mut options: Vec<BroadcastTarget> = self
            .groups
            .iter()
            .map(|g| BroadcastTarget::Group {
                id: g.id,
                label: g.name.clone(),
            })
            .collect();

        let mut tags: Vec<String> = self
            .hosts
            .iter()
            .flat_map(|h| h.tags().iter().cloned())
            .collect();
        tags.sort();
        tags.dedup();
        for name in tags {
            options.push(BroadcastTarget::Tag { name });
        }

        if options.is_empty() {
            self.host_notice = Some("No groups or tags to broadcast to.".into());
            return;
        }

        self.broadcast_setup = Some(BroadcastSetup {
            options,
            menu_selected: 0,
            target_label: String::new(),
            command: String::new(),
            cursor: 0,
            candidates: Vec::new(),
            preview_selected: 0,
            edit_targets: false,
        });
        self.mode = AppMode::BroadcastPickTarget;
    }

    /// Stage 1: the target-pick menu. Up/Down move the highlight (clamped),
    /// Enter resolves the highlighted target to candidates and advances to the
    /// command prompt, Esc closes the wizard.
    pub(crate) fn handle_key_broadcast_pick(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.broadcast_setup = None;
                self.mode = AppMode::Normal;
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => {
                if let Some(s) = self.broadcast_setup.as_mut() {
                    if s.menu_selected + 1 < s.options.len() {
                        s.menu_selected += 1;
                    }
                }
            }
            _ if self.is_action(KeyAction::MoveUp, &key) => {
                if let Some(s) = self.broadcast_setup.as_mut() {
                    s.menu_selected = s.menu_selected.saturating_sub(1);
                }
            }
            KeyCode::Enter => {
                self.resolve_broadcast_candidates();
                self.mode = AppMode::BroadcastCommand;
            }
            _ => {}
        }
        Ok(())
    }

    /// Fill `candidates` from the highlighted target: managed hosts only, kept in
    /// host-list order, each selected by default. Also derives the display
    /// `target_label` shown in the prompt / panel header.
    fn resolve_broadcast_candidates(&mut self) {
        let target = match self
            .broadcast_setup
            .as_ref()
            .and_then(|s| s.options.get(s.menu_selected))
        {
            Some(t) => t.clone(),
            None => return,
        };

        let target_label = match &target {
            BroadcastTarget::Group { label, .. } => format!("group: {label}"),
            BroadcastTarget::Tag { name } => format!("#{name}"),
        };

        let store = self.password_store.as_ref();
        let candidates: Vec<BroadcastCandidate> = self
            .hosts
            .iter()
            .filter_map(|entry| {
                let host_id = entry.managed_id()?;
                let matches = match &target {
                    BroadcastTarget::Group { id, .. } => entry.group_ids().contains(id),
                    BroadcastTarget::Tag { name } => entry.tags().iter().any(|t| t == name),
                };
                if !matches {
                    return None;
                }
                // Resolve a stored password/passphrase now (same path connect +
                // detect use); threaded into the run so password hosts auth via
                // SSH_ASKPASS instead of failing under BatchMode.
                let secret = resolve_pending_secret(entry, store).0;
                Some(BroadcastCandidate {
                    host_id,
                    host_name: entry.name().to_string(),
                    argv: ssh_argv_for_entry(entry),
                    secret,
                    selected: true,
                })
            })
            .collect();

        if let Some(s) = self.broadcast_setup.as_mut() {
            s.candidates = candidates;
            s.target_label = target_label;
            s.preview_selected = 0;
        }
    }

    /// Stage 2: single-line command entry. Mirrors the SFTP prompt's text-input
    /// idiom (char / backspace / cursor keys). Enter on a non-empty command
    /// advances to the preview barrier; Esc steps back to the target picker.
    pub(crate) fn handle_key_broadcast_command(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::BroadcastPickTarget;
            }
            KeyCode::Enter => {
                let nonempty = self
                    .broadcast_setup
                    .as_ref()
                    .is_some_and(|s| !s.command.trim().is_empty());
                if nonempty {
                    if let Some(s) = self.broadcast_setup.as_mut() {
                        s.preview_selected = 0;
                        s.edit_targets = false;
                    }
                    self.mode = AppMode::BroadcastPreview;
                }
            }
            KeyCode::Backspace if key.modifiers.is_empty() => {
                if let Some(s) = self.broadcast_setup.as_mut() {
                    s.cursor = text_input::backspace_at(&mut s.command, s.cursor);
                }
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End | KeyCode::Delete => {
                if let Some(s) = self.broadcast_setup.as_mut() {
                    let mut cursor = s.cursor;
                    text_input::handle_cursor_key(key.code, &mut s.command, &mut cursor);
                    s.cursor = cursor;
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(s) = self.broadcast_setup.as_mut() {
                    s.cursor = text_input::insert_at(&mut s.command, s.cursor, c);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Stage 3: the preview barrier. In edit-targets mode Up/Down move the row
    /// cursor and Space toggles a host's inclusion (`e`/Esc exit edit mode).
    /// Otherwise `y` starts the run, `e` enters edit mode, and `n`/`N`/Esc close
    /// the wizard.
    pub(crate) fn handle_key_broadcast_preview(&mut self, key: KeyEvent) -> Result<()> {
        let editing = self
            .broadcast_setup
            .as_ref()
            .is_some_and(|s| s.edit_targets);

        if editing {
            match key.code {
                _ if self.is_action(KeyAction::MoveDown, &key) => {
                    if let Some(s) = self.broadcast_setup.as_mut() {
                        if s.preview_selected + 1 < s.candidates.len() {
                            s.preview_selected += 1;
                        }
                    }
                }
                _ if self.is_action(KeyAction::MoveUp, &key) => {
                    if let Some(s) = self.broadcast_setup.as_mut() {
                        s.preview_selected = s.preview_selected.saturating_sub(1);
                    }
                }
                KeyCode::Char(' ') => {
                    if let Some(s) = self.broadcast_setup.as_mut() {
                        let idx = s.preview_selected;
                        if let Some(c) = s.candidates.get_mut(idx) {
                            c.selected = !c.selected;
                        }
                    }
                }
                // "done" — leave edit mode back to the [y]/[e]/[N] barrier.
                KeyCode::Enter | KeyCode::Char('e') | KeyCode::Esc => {
                    if let Some(s) = self.broadcast_setup.as_mut() {
                        s.edit_targets = false;
                    }
                }
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Char('y') => self.start_broadcast(),
                KeyCode::Char('e') => {
                    if let Some(s) = self.broadcast_setup.as_mut() {
                        s.edit_targets = true;
                    }
                }
                // Back to the command prompt to edit the command (targets kept).
                KeyCode::Char('c') => {
                    if let Some(s) = self.broadcast_setup.as_mut() {
                        s.cursor = s.command.chars().count();
                    }
                    self.mode = AppMode::BroadcastCommand;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.broadcast_setup = None;
                    self.mode = AppMode::Normal;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Launch the run: turn the selected candidates into tasks, spawn the pool,
    /// seed the result table, arm the entry slide (spawn → docked), and hand off
    /// to the background panel. Refuses (notice + close) when nothing is
    /// selected.
    fn start_broadcast(&mut self) {
        let Some(setup) = self.broadcast_setup.take() else {
            return;
        };

        let tasks: Vec<crate::broadcast::BroadcastTask> = setup
            .candidates
            .iter()
            .filter(|c| c.selected)
            .map(|c| crate::broadcast::BroadcastTask {
                host_id: c.host_id,
                host_name: c.host_name.clone(),
                argv: c.argv.clone(),
                secret: c.secret.clone(),
            })
            .collect();

        if tasks.is_empty() {
            self.host_notice = Some("No hosts selected for broadcast.".into());
            self.mode = AppMode::Normal;
            return;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let rx = crate::broadcast::spawn_broadcast(
            tasks.clone(),
            setup.command.clone(),
            crate::broadcast::DEFAULT_CONCURRENCY,
            cancel.clone(),
            Arc::new(crate::broadcast::SshCommandRunner::new()),
        );
        let results = crate::broadcast::seed_results(&tasks);

        let body =
            crate::tui::dashboard_layout::dashboard_layout_zoomed(self.terminal_area, self.ui_zoom)
                .body;
        let anim = Some(crate::tui::tween::SlideAnim::new(
            crate::tui::screens::broadcast::spawn_rect(body),
            crate::tui::screens::broadcast::docked_rect(body),
            crate::broadcast::ENTRY_ANIM,
        ));

        self.broadcast = Some(BroadcastState {
            target_label: setup.target_label,
            command: setup.command,
            results,
            rx,
            cancel,
            concurrency: crate::broadcast::DEFAULT_CONCURRENCY,
            phase: BroadcastPhase::Running,
            anim,
            audit_written: false,
        });
        self.broadcast_setup = None;
        self.mode = AppMode::Normal;
        // Deliberately do NOT focus the panel: it runs in the background and,
        // once finished, the countdown auto-dismisses it. Focusing here would
        // immediately pause that countdown (focus == "user is reading it"), so
        // the panel would never leave on its own. The user can Alt+arrow onto it
        // to inspect output, which pauses the countdown as intended.
    }

    /// Send the docked panel off with the exit slide (dock -> off the right
    /// edge) and mark it `Leaving`; `tick_broadcast` removes it once the slide
    /// finishes. Used both by the auto-dismiss countdown and by re-opening the
    /// wizard over a finished panel. No-op if there's no panel.
    fn slide_broadcast_out(&mut self) {
        let body =
            crate::tui::dashboard_layout::dashboard_layout_zoomed(self.terminal_area, self.ui_zoom)
                .body;
        if let Some(bc) = self.broadcast.as_mut() {
            let dock = crate::tui::screens::broadcast::docked_rect(body);
            let mut exit = dock;
            exit.x = body.x + body.width; // fully off to the right
            bc.anim = Some(crate::tui::tween::SlideAnim::new(
                dock,
                exit,
                crate::broadcast::ENTRY_ANIM,
            ));
            bc.phase = BroadcastPhase::Leaving;
        }
        if self.focused_panel == PanelId::Broadcast {
            self.focused_panel = PanelId::default();
            self.panel_zoomed = false;
        }
    }

    /// Poll-loop step for a live run: drain worker events, fold them into the
    /// row table, retire the entry animation, arm the completion countdown,
    /// write the one-shot audit trail, and drive settle/pause/dismiss.
    pub(crate) fn tick_broadcast(&mut self) -> Result<()> {
        // Capture the clock before the &mut borrows below (the phase timestamps
        // read it, and the borrow checker won't let us call it mid-borrow).
        let now = Instant::now();

        // Expire error toasts whose slide-out has finished. Runs even with no
        // live panel — a 10s toast can outlive the 6.5s panel.
        self.broadcast_toasts.retain(|t| {
            t.born.elapsed() < crate::broadcast::TOAST_TTL + crate::broadcast::TOAST_ANIM
        });

        if self.broadcast.is_none() {
            return Ok(());
        }

        // Drain the worker channel and fold each event into the result rows.
        // Retire the entry slide once it finishes, arm the countdown the first
        // tick every row is terminal, and spawn a toast per failed host.
        let mut new_toasts: Vec<BroadcastToast> = Vec::new();
        if let Some(bc) = self.broadcast.as_mut() {
            let events: Vec<crate::broadcast::BroadcastEvent> =
                std::iter::from_fn(|| bc.rx.try_recv().ok()).collect();
            for ev in &events {
                crate::broadcast::apply_event(&mut bc.results, ev);
            }
            for ev in &events {
                let (host_id, text) = match ev {
                    crate::broadcast::BroadcastEvent::Finished {
                        host_id,
                        exit,
                        stderr,
                        ..
                    } if *exit != 0 => {
                        let t = stderr
                            .lines()
                            .map(str::trim)
                            .find(|l| !l.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("exit {exit}"));
                        (*host_id, t)
                    }
                    crate::broadcast::BroadcastEvent::Failed { host_id, reason } => {
                        (*host_id, reason.clone())
                    }
                    _ => continue,
                };
                let host = bc
                    .results
                    .iter()
                    .find(|r| r.host_id == host_id)
                    .map(|r| r.host_name.clone())
                    .unwrap_or_default();
                new_toasts.push(BroadcastToast {
                    host,
                    text,
                    born: now,
                });
            }
            if bc.anim.as_ref().is_some_and(|a| a.is_done(now)) {
                bc.anim = None;
            }
            if bc.phase == BroadcastPhase::Running && crate::broadcast::all_terminal(&bc.results) {
                bc.phase = BroadcastPhase::Settling { done_at: now };
            }
        }
        self.broadcast_toasts.extend(new_toasts);

        // Audit once, at completion. Split the borrow: gather rows while holding
        // &mut self.broadcast, flip the guard, then drop the borrow before
        // touching self.store.
        let mut audit_rows: Vec<(String, String, String)> = Vec::new();
        if let Some(bc) = self.broadcast.as_mut() {
            if !bc.audit_written && crate::broadcast::all_terminal(&bc.results) {
                for r in &bc.results {
                    // Use the canonical audit status vocabulary so broadcast
                    // rows integrate with the stats query, the Ok/Fail audit
                    // filters, and theme::status_color: "launched" (green/Ok)
                    // for a clean exit, "fail" (red/Fail) otherwise.
                    let (status, note) = match &r.state {
                        crate::broadcast::HostState::Done { exit } => {
                            let status = if *exit == 0 { "launched" } else { "fail" };
                            // On failure, fold the first stderr line into the note
                            // so the Audit tab records WHAT broke, not just the code.
                            let note = match crate::broadcast::error_text(r) {
                                Some(err) => format!("{} (exit {}: {})", bc.command, exit, err),
                                None => format!("{} (exit {})", bc.command, exit),
                            };
                            (status.to_string(), note)
                        }
                        crate::broadcast::HostState::Failed { reason } => {
                            ("fail".to_string(), format!("{} ({})", bc.command, reason))
                        }
                        // Unreachable once all_terminal holds; stay total anyway.
                        _ => continue,
                    };
                    audit_rows.push((r.host_name.clone(), status, note));
                }
                bc.audit_written = true;
            }
        }
        for (host, status, note) in audit_rows {
            let _ = self
                .store
                .log_auth_event(&host, None, "broadcast", &status, &note, None);
        }

        // Pause the countdown while the panel is focused (zoom keeps it focused);
        // resume it when focus leaves; once it elapses, play the exit slide and
        // remove the panel when the slide finishes.
        let focused = self.focused_panel == PanelId::Broadcast;
        let mut start_exit = false;
        let mut dismiss = false;
        if let Some(bc) = self.broadcast.as_mut() {
            match bc.phase {
                BroadcastPhase::Settling { done_at } => {
                    if focused {
                        bc.phase = BroadcastPhase::Paused;
                    } else if done_at.elapsed() >= crate::broadcast::DISMISS {
                        start_exit = true;
                    }
                }
                BroadcastPhase::Paused => {
                    if !focused {
                        bc.phase = BroadcastPhase::Settling { done_at: now };
                    }
                }
                BroadcastPhase::Leaving => {
                    // anim is retired above once done; remove the panel then.
                    if bc.anim.is_none() {
                        dismiss = true;
                    }
                }
                BroadcastPhase::Running => {}
            }
        }
        if start_exit {
            self.slide_broadcast_out();
        }
        if dismiss {
            self.broadcast = None;
            if self.focused_panel == PanelId::Broadcast {
                self.focused_panel = PanelId::default();
                self.panel_zoomed = false;
            }
        }
        Ok(())
    }

    /// Signal the run to cancel: workers finish killing in-flight children and
    /// mark the rest `Failed{cancelled}`, which `tick_broadcast` then folds in.
    pub(crate) fn cancel_broadcast(&mut self) {
        if let Some(bc) = &self.broadcast {
            bc.cancel.store(true, Ordering::Relaxed);
        }
    }
}
