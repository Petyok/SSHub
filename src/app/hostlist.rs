use super::*;

/// Time constant of the host list's scroll smoothing (#35): the position covers
/// ~63% of the distance left to its target every `HOST_SCROLL_TAU`.
const HOST_SCROLL_TAU: f32 = 0.055;

/// Time constant of the header counters' chase (#35), a touch slower than the
/// list scroll so a jump of a few hosts is legible as it counts.
const HEADER_STATS_TAU: f32 = 0.09;

impl App {
    /// Map a Y offset (relative to hosts panel content area) to a host index,
    /// accounting for group headers and blank separators.
    /// Flattened host-tree layout: total visual rows (group headers + blank
    /// separators + host rows) and the visual row of the selected host.
    pub fn host_visual_layout(&self) -> (usize, Option<usize>) {
        let rows = self.host_visual_rows();
        let sel = rows.iter().position(|r| {
            matches!(
                r,
                VisualRow::Header { selected: true, .. } | VisualRow::Host { selected: true, .. }
            )
        });
        (rows.len(), sel)
    }

    /// Scroll offset (in visual rows) for a host panel of `body_h` rows that
    /// keeps the selected host roughly centered and on screen.
    pub fn host_scroll_offset(&self, body_h: usize) -> usize {
        if body_h == 0 {
            return 0;
        }
        let (total, sel) = self.host_visual_layout();
        let max_offset = total.saturating_sub(body_h);
        match sel {
            Some(s) => s.saturating_sub(body_h / 2).min(max_offset),
            None => 0,
        }
    }

    /// Advance the host list's smoothed scroll position toward the target
    /// offset and return the row offset to draw at (#35).
    ///
    /// The list is drawn on whole rows, so the slide can't be a sub-row shift;
    /// instead the position chases its target exponentially and each frame
    /// lands on the nearest row, which reads as the list scrolling rather than
    /// jumping half a panel whenever the selection leaves the window. Called
    /// once per frame from the render pass (hence `Cell`, not `&mut self`);
    /// everything else reads [`App::host_scroll_shown`] so click mapping stays
    /// on the row the user actually sees.
    pub(crate) fn host_scroll_advance(&self, body_h: usize) -> usize {
        let target = self.host_scroll_offset(body_h);
        if !self.motion_enabled() {
            self.host_scroll_pos.set(target as f32);
            self.host_scroll_moving.set(false);
            return target;
        }
        let now = std::time::Instant::now();
        let Some(last) = self.host_scroll_at.get() else {
            // First frame: start where the list already is, don't scroll in.
            self.host_scroll_pos.set(target as f32);
            self.host_scroll_at.set(Some(now));
            self.host_scroll_moving.set(false);
            return target;
        };
        self.host_scroll_at.set(Some(now));
        let pos = self.host_scroll_pos.get();
        let dist = target as f32 - pos;
        // Within half a row of the target: settle exactly, so the list can come
        // to rest and stop asking the loop for 60fps.
        if dist.abs() < 0.5 {
            self.host_scroll_pos.set(target as f32);
            self.host_scroll_moving.set(false);
            return target;
        }
        // Exponential approach: covers ~63% of the remaining distance per
        // HOST_SCROLL_TAU, so a one-row nudge and a half-panel jump both take
        // about the same (short) time and neither overshoots.
        let dt = now.saturating_duration_since(last).as_secs_f32();
        let k = 1.0 - (-dt / HOST_SCROLL_TAU).exp();
        let next = pos + dist * k;
        self.host_scroll_pos.set(next);
        self.host_scroll_moving.set(true);
        next.round().max(0.0) as usize
    }

    /// The row offset the host list is currently drawn at. Mirrors whatever
    /// [`App::host_scroll_advance`] last settled on, so hit-testing agrees with
    /// what is on screen mid-scroll.
    pub(crate) fn host_scroll_shown(&self, body_h: usize) -> usize {
        if !self.motion_enabled() || self.host_scroll_at.get().is_none() {
            return self.host_scroll_offset(body_h);
        }
        self.host_scroll_pos.get().round().max(0.0) as usize
    }

    /// Scroll offset, in whole card-rows, for the keys tab. Keeps the selected
    /// identity card on screen (roughly centered) when the grid overflows.
    /// `card_row_stride` is the height of one card row (card height + gap).
    pub fn keys_scroll_row_offset(
        &self,
        area_height: u16,
        cards_per_row: usize,
        card_row_stride: u16,
    ) -> usize {
        let cpr = cards_per_row.max(1);
        let stride = card_row_stride.max(1) as usize;
        let total_rows = self.identities.len().div_ceil(cpr);
        let visible_rows = ((area_height as usize) / stride).max(1);
        let selected_row = self.identity_selected / cpr;
        let max_off = total_rows.saturating_sub(visible_rows);
        selected_row.saturating_sub(visible_rows / 2).min(max_off)
    }

    /// Advance the identities grid toward `target_lines` and return the line
    /// offset to draw at (#35). Same chase as the host list, but measured in
    /// lines rather than card rows, so a card-row jump scrolls through instead
    /// of teleporting a whole card height.
    pub(crate) fn keys_scroll_advance(&self, target_lines: usize) -> u16 {
        let goal = target_lines as f32;
        if !self.motion_enabled() {
            self.keys_scroll_pos.set(goal);
            self.keys_scroll_moving.set(false);
            return target_lines as u16;
        }
        let now = std::time::Instant::now();
        let last = self.keys_scroll_at.replace(Some(now));
        let pos = self.keys_scroll_pos.get();
        let dist = goal - pos;
        if last.is_none() || dist.abs() < 0.5 {
            self.keys_scroll_pos.set(goal);
            self.keys_scroll_moving.set(false);
            return target_lines as u16;
        }
        let dt = now.saturating_duration_since(last.unwrap()).as_secs_f32();
        let next = pos + dist * (1.0 - (-dt / HOST_SCROLL_TAU).exp());
        self.keys_scroll_pos.set(next);
        self.keys_scroll_moving.set(true);
        next.round().max(0.0) as u16
    }

    /// Map a click at visible row `rel_y` (within a `body_h`-row panel) to the
    /// host index under it, accounting for the current scroll offset.
    pub(crate) fn host_row_to_index(&self, rel_y: u16, body_h: usize) -> Option<usize> {
        let target = rel_y as usize + self.host_scroll_shown(body_h);
        match self.host_visual_rows().get(target) {
            Some(VisualRow::Host { host_idx, .. }) => Some(*host_idx),
            _ => None,
        }
    }

    /// Map a click at visible row `rel_y` to a group-header section index (for
    /// click-to-collapse), accounting for the current scroll offset.
    pub(crate) fn host_row_to_header(&self, rel_y: u16, body_h: usize) -> Option<usize> {
        let target = rel_y as usize + self.host_scroll_shown(body_h);
        match self.host_visual_rows().get(target) {
            Some(VisualRow::Header { section, .. }) => Some(*section),
            _ => None,
        }
    }

    pub fn selected_host_index(&self) -> Option<usize> {
        match self.nav_rows.get(self.selected) {
            Some(NavRow::Host(i)) => Some(*i),
            _ => None,
        }
    }

    /// The full rendered layout of the hosts tree: blank separators, group
    /// headers and host rows, with per-row selection state. Single source of
    /// truth shared by rendering, scroll math and click mapping.
    pub fn host_visual_rows(&self) -> Vec<VisualRow> {
        // Driven by `nav_rows` (the single source of truth for what's visible
        // and navigable), so hidden subtrees never leak host rows. Blank
        // separators go before each top-level header (except the first row);
        // nested headers sit flush under their parent. `depth` drives indent.
        let mut rows = Vec::new();
        let mut cur_host_depth = 1usize;
        let mut first = true;
        // A fold in flight (#35) shows only part of its group's subtree: the
        // rows past that point are simply left out, so the list below closes up
        // behind them a row at a time instead of jumping.
        let reveal = self.fold_reveal();
        // (rows still allowed through, depth of the folding header)
        let mut budget: Option<(usize, usize)> = None;
        for (nav_idx, row) in self.nav_rows.iter().enumerate() {
            // Leaving the folding group's subtree ends the budget.
            if let (Some((_, depth)), NavRow::Header(si)) = (budget, *row) {
                if self.group_sections[si].depth <= depth {
                    budget = None;
                }
            }
            if let Some((left, _)) = budget.as_mut() {
                if *left == 0 {
                    continue;
                }
                *left -= 1;
            }
            match *row {
                NavRow::Header(si) => {
                    let section = &self.group_sections[si];
                    if !first && section.depth == 0 {
                        rows.push(VisualRow::Blank);
                    }
                    rows.push(VisualRow::Header {
                        section: si,
                        collapsed: section.collapsed,
                        selected: self.selected == nav_idx,
                        depth: section.depth,
                    });
                    cur_host_depth = section.depth + 1;
                    if let Some((anim, shown)) = reveal {
                        if section.key() == anim.key {
                            if anim.expanding {
                                // Unfolding: the subtree is live in `nav_rows`,
                                // so let a growing prefix of it through.
                                let total = self
                                    .nav_rows
                                    .iter()
                                    .skip(nav_idx + 1)
                                    .take_while(|r| match r {
                                        NavRow::Header(s) => {
                                            self.group_sections[*s].depth > section.depth
                                        }
                                        NavRow::Host(_) => true,
                                    })
                                    .count();
                                budget =
                                    Some(((total as f32 * shown).round() as usize, section.depth));
                            } else {
                                // Folding: the subtree is already gone from
                                // `nav_rows`, so replay a shrinking prefix of
                                // what it looked like. These rows are display
                                // only — nothing navigates to them.
                                let keep = (anim.rows.len() as f32 * shown).round() as usize;
                                rows.extend(anim.rows.iter().take(keep).cloned());
                            }
                        }
                    }
                }
                NavRow::Host(host_idx) => {
                    rows.push(VisualRow::Host {
                        host_idx,
                        selected: self.selected == nav_idx,
                        // Flat (no-group) lists have no headers → depth 0.
                        depth: if self.groups.is_empty() {
                            0
                        } else {
                            cur_host_depth
                        },
                    });
                }
            }
            first = false;
        }
        rows
    }

    pub fn selected_entry(&self) -> Option<&HostEntry> {
        let host_idx = self.selected_host_index()?;
        self.hosts.get(host_idx)
    }

    /// The section index if the current selection is a group header.
    pub fn selected_nav_header(&self) -> Option<usize> {
        match self.nav_rows.get(self.selected) {
            Some(NavRow::Header(si)) => Some(*si),
            _ => None,
        }
    }

    pub(crate) fn load_collapsed_groups(&mut self) {
        if let Ok(Some(raw)) = self.store.get_ui_state("collapsed_groups") {
            if let Ok(ids) = serde_json::from_str::<Vec<i64>>(&raw) {
                self.collapsed_groups = ids.into_iter().collect();
            }
        }
    }

    pub(crate) fn persist_collapsed_groups(&self) {
        let mut ids: Vec<i64> = self.collapsed_groups.iter().copied().collect();
        ids.sort_unstable();
        if let Ok(json) = serde_json::to_string(&ids) {
            let _ = self.store.set_ui_state("collapsed_groups", &json);
        }
    }

    /// Toggle collapse of the group header under the selection, keeping the
    /// selection on that header, and persist the new state.
    pub(crate) fn toggle_selected_group(&mut self) {
        if let Some(si) = self.selected_nav_header() {
            self.toggle_group_by_section(si);
        }
    }

    pub(crate) fn toggle_group_by_section(&mut self, si: usize) {
        // Any fold still playing is stale the moment the tree changes again.
        self.fold_anim = None;
        let Some(section) = self.group_sections.get(si) else {
            return;
        };
        let key = section.key();
        let expanding = self.collapsed_groups.contains(&key);
        // Capture the rows about to disappear *before* collapsing, so the fold
        // has something to swallow. An unfold needs no capture: its rows are in
        // `nav_rows` the moment the collapse is lifted.
        let rows = if expanding {
            Vec::new()
        } else {
            self.subtree_visual_rows(key)
        };
        if self.motion_enabled() {
            self.fold_anim = Some(FoldAnim {
                key,
                expanding,
                at: std::time::Instant::now(),
                rows,
            });
        }
        if expanding {
            self.collapsed_groups.remove(&key);
        } else {
            self.collapsed_groups.insert(key);
        }
        self.persist_collapsed_groups();
        self.rebuild_filter();
        if let Some(pos) = self
            .nav_rows
            .iter()
            .position(|r| matches!(r, NavRow::Header(s) if self.group_sections[*s].key() == key))
        {
            self.selected = pos;
        }
    }

    /// Advance the header counters toward `target` and return what to draw
    /// (#35). Each counter closes on its real value instead of snapping, so a
    /// handful of hosts dropping offline reads as the tally moving rather than
    /// a number blinking. Same chase as the host list's scroll, and likewise
    /// called once per frame from the render pass.
    pub(crate) fn header_stats_advance(&self, target: [usize; 4]) -> [usize; 4] {
        let goal = target.map(|v| v as f32);
        if !self.motion_enabled() {
            self.header_stats_pos.set(goal);
            self.header_stats_moving.set(false);
            return target;
        }
        let now = std::time::Instant::now();
        let Some(last) = self.header_stats_at.get() else {
            // First frame: the tally starts where it really is.
            self.header_stats_pos.set(goal);
            self.header_stats_at.set(Some(now));
            self.header_stats_moving.set(false);
            return target;
        };
        self.header_stats_at.set(Some(now));
        let dt = now.saturating_duration_since(last).as_secs_f32();
        let k = 1.0 - (-dt / HEADER_STATS_TAU).exp();
        let mut pos = self.header_stats_pos.get();
        let mut out = [0usize; 4];
        let mut moving = false;
        for i in 0..4 {
            let dist = goal[i] - pos[i];
            if dist.abs() < 0.5 {
                pos[i] = goal[i];
                out[i] = target[i];
            } else {
                pos[i] += dist * k;
                out[i] = pos[i].round().max(0.0) as usize;
                moving = true;
            }
        }
        self.header_stats_pos.set(pos);
        self.header_stats_moving.set(moving);
        out
    }

    /// Notice hosts whose ping class changed since the last tick and stamp
    /// them, so their status dot can flash on the way to its new colour (#35).
    /// A host seen for the first time is stamped as already settled: opening
    /// the app shouldn't set the whole list flashing.
    pub(crate) fn detect_ping_changes(&mut self) {
        let now = std::time::Instant::now();
        let settled = now.checked_sub(crate::tui::PING_FLASH).unwrap_or(now);
        for (name, samples) in &self.ping_data {
            // A new reading grows into the sparkline rather than popping in.
            if let Some(last) = samples.last().copied() {
                match self.ping_sample.get(name) {
                    Some((prev, _)) if *prev == last => {}
                    Some(_) => {
                        self.ping_sample.insert(name.clone(), (last, now));
                    }
                    None => {
                        self.ping_sample.insert(name.clone(), (last, settled));
                    }
                }
            }
            let class = crate::ping::classify_ping(Some(samples));
            match self.ping_flash.get(name) {
                Some((prev, _)) if *prev == class => continue,
                Some(_) => {
                    self.ping_flash.insert(name.clone(), (class, now));
                }
                None => {
                    self.ping_flash.insert(name.clone(), (class, settled));
                }
            }
        }
    }

    /// How far a host's newest sparkline column has grown, `0.0` to `1.0`
    /// (#35). `1.0` at rest, so a settled graph draws at full height.
    pub(crate) fn ping_grow(&self, host: &str) -> f32 {
        if !self.motion_enabled() {
            return 1.0;
        }
        let Some((_, at)) = self.ping_sample.get(host) else {
            return 1.0;
        };
        crate::tui::tween::ease_out(crate::tui::tween::progress(
            *at,
            crate::tui::PING_FLASH,
            std::time::Instant::now(),
        ))
    }

    /// Colour for a host's status dot: its settled class colour, flashed
    /// through white for [`crate::tui::PING_FLASH`] after the class changes
    /// (#35), so a host dropping offline catches the eye.
    pub(crate) fn ping_flash_color(
        &self,
        host: &str,
        settled: ratatui::style::Color,
    ) -> ratatui::style::Color {
        if !self.motion_enabled() {
            return settled;
        }
        let Some((_, at)) = self.ping_flash.get(host) else {
            return settled;
        };
        let p = crate::tui::tween::progress(*at, crate::tui::PING_FLASH, std::time::Instant::now());
        if p >= 1.0 {
            return settled;
        }
        // The flash peaks on the *active* theme's brightest text, not the
        // frozen legacy constant — a dark-on-light theme flashes to its own
        // highlight instead of punching a light grey hole in the row.
        //
        // Read from the `text.bright` **role**, not from `semantic.text_bright`:
        // the two coincide under `default`, but a theme overriding the role has
        // to reach the flash too. The role's `Style` is only guaranteed a
        // foreground when it sets one, so the token is the documented fallback.
        let theme = self.theme();
        let peak = theme
            .style(crate::theme::catalog::StyleRole::TextBright)
            .fg
            .unwrap_or(theme.semantic().text_bright);
        crate::tui::tween::color_lerp(peak, settled, crate::tui::tween::ease_out(p))
    }

    /// The visual rows of the group's subtree as it stands right now: what a
    /// fold is about to swallow, captured so it can be replayed on the way out.
    fn subtree_visual_rows(&self, key: i64) -> Vec<VisualRow> {
        let rows = self.host_visual_rows();
        let Some(head) = rows.iter().position(|r| {
            matches!(r, VisualRow::Header { section, .. }
                if self.group_sections[*section].key() == key)
        }) else {
            return Vec::new();
        };
        let VisualRow::Header { depth, .. } = rows[head] else {
            return Vec::new();
        };
        rows.into_iter()
            .skip(head + 1)
            .take_while(|r| match r {
                VisualRow::Header { depth: d, .. } => *d > depth,
                VisualRow::Host { .. } => true,
                VisualRow::Blank => false,
            })
            .collect()
    }

    /// Fraction of the animating group's subtree that is on screen right now
    /// (#35): it grows as the group opens and shrinks as it shuts. `None` when
    /// no fold is playing, or once it has run its course.
    fn fold_reveal(&self) -> Option<(&FoldAnim, f32)> {
        let anim = self.fold_anim.as_ref()?;
        if !self.motion_enabled() {
            return None;
        }
        let p =
            crate::tui::tween::progress(anim.at, crate::tui::FOLD_ANIM, std::time::Instant::now());
        if p >= 1.0 {
            return None;
        }
        // Symmetric easing: a fold and the unfold that undoes it should feel
        // like the same motion run backwards, which an ease-out does not.
        let e = crate::tui::tween::ease_in_out(p);
        Some((anim, if anim.expanding { e } else { 1.0 - e }))
    }

    /// Collapse (`false`) or expand (`true`) every group at once.
    pub(crate) fn set_all_groups_collapsed(&mut self, collapsed: bool) {
        // A per-group fold in flight would fight the bulk change; drop it.
        self.fold_anim = None;
        if collapsed {
            self.collapsed_groups = self.group_sections.iter().map(|s| s.key()).collect();
        } else {
            self.collapsed_groups.clear();
        }
        self.persist_collapsed_groups();
        let sel_key = self
            .selected_nav_header()
            .map(|si| self.group_sections[si].key());
        self.rebuild_filter();
        if let Some(key) = sel_key {
            if let Some(pos) = self.nav_rows.iter().position(
                |r| matches!(r, NavRow::Header(s) if self.group_sections[*s].key() == key),
            ) {
                self.selected = pos;
            }
        }
    }

    pub(crate) fn toggle_favorite(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };

        if let HostEntry::Managed(m) = &self.hosts[host_idx] {
            let id = m.id;
            let new_fav = !m.favorite;
            // Favourite status == membership in the reserved Favorites group.
            let favorites_id = self.store.favorites_group_id()?;
            if new_fav {
                self.store.add_host_to_group(id, favorites_id)?;
            } else {
                self.store.remove_host_from_group(id, favorites_id)?;
            }
            // Keep the legacy column in sync for back-compat (reads come from
            // membership) and reload so `groups`/`favorite` reflect the change.
            let _ = self.store.update_host(
                id,
                &HostUpdate {
                    favorite: Some(new_fav),
                    ..Default::default()
                },
            );
            self.reload_hosts()?;
            return Ok(());
        }

        let host_name = self.hosts[host_idx].name().to_string();
        self.metadata.toggle_favorite(&host_name)?;
        if let Some((_, meta)) = self.hosts[host_idx].legacy_mut() {
            if let Some(stored) = self.metadata.get(&host_name)? {
                meta.favorite = stored.favorite;
            }
        }
        Ok(())
    }

    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.next();
        self.rebuild_filter();
    }

    pub(crate) fn move_host_manual(&mut self, delta: i32) -> Result<()> {
        if self.sort_mode != SortMode::Manual {
            return Ok(());
        }
        let Some(id) = self.selected_entry().and_then(|e| e.managed_id()) else {
            return Ok(());
        };
        let name = self.selected_entry().map(|e| e.name().to_string());
        // Find the adjacent *host* nav row in the requested direction (skip
        // group headers so manual reorder only swaps hosts).
        let mut probe = self.selected as i32 + delta;
        let other_idx = loop {
            if probe < 0 || probe >= self.nav_rows.len() as i32 {
                return Ok(());
            }
            match self.nav_rows[probe as usize] {
                NavRow::Host(i) => break i,
                NavRow::Header(_) => probe += delta,
            }
        };
        let Some(other_id) = self.hosts[other_idx].managed_id() else {
            return Ok(());
        };

        self.store.swap_host_sort_orders(id, other_id)?;
        self.reload_hosts()?;
        if let Some(name) = name {
            self.restore_selection_by_name(&name);
        }
        Ok(())
    }

    pub(crate) fn rebuild_filter(&mut self) {
        let candidates: Vec<usize> = if self.tag_filters.is_empty() {
            (0..self.hosts.len()).collect()
        } else {
            self.hosts
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    let tags = entry.tags();
                    // AND: the host must carry every selected tag.
                    self.tag_filters.iter().all(|f| tags.iter().any(|t| t == f))
                })
                .map(|(idx, _)| idx)
                .collect()
        };

        let entries: Vec<HostEntry> = candidates
            .iter()
            .map(|&idx| self.hosts[idx].clone())
            .collect();
        let local_matches = self.search.update_query(&entries, &self.search_query);
        let mut filtered: Vec<usize> = local_matches
            .into_iter()
            .map(|local_idx| candidates[local_idx])
            .collect();

        sort_host_indices(&self.hosts, &mut filtered, self.sort_mode);
        // Partition by group, then flatten back so filtered_indices walks in
        // visual order. Within each section the existing sort_mode order is
        // preserved by build_group_sections. Without this, j/k steps through
        // the alphabetical list while the screen shows grouped sections, so
        // moving past a grouped host visually "teleports" to the group at the
        // top of the list and back.
        self.group_sections = build_group_sections(&self.hosts, &self.tree_groups(), &filtered);
        // While a filter is active, drop groups whose whole subtree has no
        // matching hosts — but keep a parent that itself is empty when a
        // descendant still matches, so nested results stay reachable.
        let filtering = !self.tag_filters.is_empty() || !self.search_query.is_empty();
        if filtering {
            let keep = subtree_has_hosts(&self.group_sections);
            let mut it = keep.into_iter();
            self.group_sections.retain(|_| it.next().unwrap_or(false));
        }
        self.filtered_indices = self
            .group_sections
            .iter()
            .flat_map(|s| s.host_indices.iter().copied())
            .collect();

        // Where the query landed inside each surviving row's display name, so
        // the list can mark it. Done here, once per rebuild, because the
        // renderer only holds `&App` and the matcher needs `&mut`.
        self.search_matches.clear();
        if !self.search_query.is_empty() {
            let query = self.search_query.clone();
            for &idx in &self.filtered_indices {
                let name = self.hosts[idx].display_name().to_string();
                let hits = self.search.display_match_indices(&name, &query);
                if !hits.is_empty() {
                    self.search_matches.insert(idx, hits);
                }
            }
        }

        // Tree mode (navigable, collapsible headers) kicks in only once there's
        // a real group section to show — a pure ssh_config list stays flat. The
        // always-present reserved Favorites group doesn't count until it has
        // members (build_group_sections hides it while empty), so we key on the
        // built sections, not the raw group list.
        // Collapsing a group hides its hosts AND its whole descendant subtree;
        // `hidden_below` tracks the depth of the nearest collapsed ancestor.
        let tree_mode = self.group_sections.iter().any(|s| s.group.is_some());
        let mut nav = Vec::new();
        let mut hidden_below: Option<usize> = None;
        for (si, section) in self.group_sections.iter_mut().enumerate() {
            let depth = section.depth;
            if let Some(cd) = hidden_below {
                if depth <= cd {
                    hidden_below = None; // left the collapsed subtree
                }
            }
            section.collapsed = tree_mode && self.collapsed_groups.contains(&section.key());
            if hidden_below.is_some() {
                continue; // an ancestor is collapsed: skip header and hosts
            }
            if tree_mode {
                nav.push(NavRow::Header(si));
            }
            if section.collapsed {
                hidden_below = Some(depth);
            } else {
                nav.extend(section.host_indices.iter().map(|&h| NavRow::Host(h)));
            }
        }
        self.nav_rows = nav;
        self.clamp_selected();
    }

    pub(crate) fn clamp_selected(&mut self) {
        if self.nav_rows.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.nav_rows.len() {
            self.selected = self.nav_rows.len() - 1;
        }
    }

    /// Make the host at `idx` (an index into [`App::hosts`]) the current
    /// selection: drop any tag/search filter that hides it, expand its group if
    /// collapsed, and point `selected` at its navigation row. Returns whether
    /// the host is now selectable (found in `nav_rows`).
    ///
    /// Used by quick-connect, where the fuzzy palette can pick a host that the
    /// current filter or a collapsed group would otherwise hide.
    pub fn reveal_host(&mut self, idx: usize) -> bool {
        if idx >= self.hosts.len() {
            return false;
        }
        // A tag/search filter may hide the chosen host — drop it rather than
        // silently landing on a different row.
        if !self.filtered_indices.contains(&idx) {
            self.tag_filters.clear();
            self.search_query.clear();
            self.rebuild_filter();
        }
        // Expand EVERY group the host belongs to (including Favorites) plus each
        // one's ancestor chain, so its row is navigable wherever it lives (a
        // collapsed ancestor hides the whole subtree). A host with no group
        // memberships lives in the ungrouped bucket.
        let mut changed = false;
        let group_ids = self
            .hosts
            .get(idx)
            .map(|h| h.group_ids())
            .unwrap_or_default();
        if group_ids.is_empty() {
            changed |= self.collapsed_groups.remove(&UNGROUPED_KEY);
        } else {
            for gid in group_ids {
                let mut group = Some(gid);
                while let Some(id) = group {
                    changed |= self.collapsed_groups.remove(&id);
                    group = self
                        .groups
                        .iter()
                        .find(|g| g.id == id)
                        .and_then(|g| g.parent_id);
                }
            }
        }
        if changed {
            self.persist_collapsed_groups();
            self.rebuild_filter();
        }
        if let Some(pos) = self
            .nav_rows
            .iter()
            .position(|r| matches!(r, NavRow::Host(i) if *i == idx))
        {
            self.selected = pos;
            true
        } else {
            false
        }
    }

    pub(crate) fn restore_selection_by_name(&mut self, name: &str) {
        let host_idx = self.hosts.iter().position(|h| h.name() == name);
        if let Some(hi) = host_idx {
            if let Some(pos) = self
                .nav_rows
                .iter()
                .position(|r| matches!(r, NavRow::Host(i) if *i == hi))
            {
                self.selected = pos;
                return;
            }
        }
        self.clamp_selected();
    }
}
